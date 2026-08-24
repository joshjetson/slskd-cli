//! TUI state and the actions the key handler drives.

use crate::{
    client::Client,
    models::{flatten as flatten_transfers, Application, BrowseDirectory, Candidate, Search, Transfer, TransferUser},
    rank::{self, Filter},
};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Search,
    Transfers,
    Browse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Editing,
}

pub struct App {
    pub api: Client,
    pub tab: Tab,
    pub mode: Mode,

    pub query: String,
    pub results: Vec<Candidate>,
    pub search_selected: usize,
    pub any_format: bool,
    pub allow_variants: bool,

    pub transfers: Vec<Transfer>,
    pub transfer_selected: usize,
    /// Transfer state keyed by (peer, exact remote path), so a search result
    /// can show what happened to that precise file.
    transfer_by_file: HashMap<(String, String), Transfer>,
    /// Basenames that completed successfully from *any* peer. Lets a result be
    /// marked as already-held even when it was fetched from someone else.
    have_basenames: HashSet<String>,

    pub browse_user: String,
    pub browse_dirs: Vec<BrowseDirectory>,
    pub browse_selected: usize,

    pub app_state: Option<Application>,
    pub status: String,

    /// A search runs on its own task so the interface keeps drawing and keeps
    /// answering keys while it waits. A Soulseek search takes about 35 seconds;
    /// awaiting it inline froze the whole app for that long, which is
    /// indistinguishable from a crash.
    pub search_task: Option<JoinHandle<anyhow::Result<Search>>>,
    pub search_started: Option<Instant>,
    pub transfers_task: Option<JoinHandle<anyhow::Result<Vec<TransferUser>>>>,
}

impl App {
    pub fn new(api: Client) -> Self {
        Self {
            api,
            tab: Tab::Search,
            mode: Mode::Normal,
            query: String::new(),
            results: Vec::new(),
            search_selected: 0,
            any_format: false,
            allow_variants: false,
            transfers: Vec::new(),
            transfer_selected: 0,
            transfer_by_file: HashMap::new(),
            have_basenames: HashSet::new(),
            browse_user: String::new(),
            browse_dirs: Vec::new(),
            browse_selected: 0,
            app_state: None,
            status: "press / to search, 1-3 to switch tabs, q to quit".into(),
            search_task: None,
            search_started: None,
            transfers_task: None,
        }
    }

    pub fn searching(&self) -> bool {
        self.search_task.is_some()
    }

    /// How long the in-flight search has been running, for the progress hint.
    pub fn search_elapsed(&self) -> Option<Duration> {
        self.search_started.map(|t| t.elapsed())
    }

    pub fn next_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Search => Tab::Transfers,
            Tab::Transfers => Tab::Browse,
            Tab::Browse => Tab::Search,
        };
    }

    fn len_for_tab(&self) -> usize {
        match self.tab {
            Tab::Search => self.results.len(),
            Tab::Transfers => self.transfers.len(),
            Tab::Browse => self.browse_dirs.len(),
        }
    }

    fn selected_mut(&mut self) -> &mut usize {
        match self.tab {
            Tab::Search => &mut self.search_selected,
            Tab::Transfers => &mut self.transfer_selected,
            Tab::Browse => &mut self.browse_selected,
        }
    }

    pub fn move_selection(&mut self, delta: i64) {
        let len = self.len_for_tab();
        if len == 0 {
            return;
        }
        let sel = self.selected_mut();
        let next = (*sel as i64 + delta).clamp(0, len as i64 - 1);
        *sel = next as usize;
    }

    pub fn select_first(&mut self) {
        *self.selected_mut() = 0;
    }

    pub fn select_last(&mut self) {
        let len = self.len_for_tab();
        if len > 0 {
            *self.selected_mut() = len - 1;
        }
    }

    pub fn toggle_variants(&mut self) {
        self.allow_variants = !self.allow_variants;
        self.status = if self.allow_variants {
            "including remixes, covers and live cuts".into()
        } else {
            "originals only — remixes and covers filtered".into()
        };
    }

    pub fn toggle_any_format(&mut self) {
        self.any_format = !self.any_format;
        self.status = if self.any_format {
            "format: any audio".into()
        } else {
            "format: mp3 only".into()
        };
    }

    fn filter(&self) -> Filter {
        Filter {
            title_tokens: rank::tokenize(&self.query),
            title_phrase: Some(rank::normalize_phrase(&self.query)),
            artist_tokens: Vec::new(),
            extensions: if self.any_format {
                Vec::new()
            } else {
                vec!["mp3".into()]
            },
            min_bitrate: 0,
            check_duration: true,
            reject_variants: !self.allow_variants,
        }
    }

    pub async fn refresh_status(&mut self) {
        match self.api.application().await {
            Ok(a) => self.app_state = Some(a),
            Err(e) => self.status = format!("status failed: {e}"),
        }
    }

    /// Ask for fresh transfers in the background. Cheap to call on a timer:
    /// if one request is still outstanding this does nothing, so a slow link
    /// cannot pile up requests or stall the frame loop.
    pub fn refresh_transfers(&mut self) {
        if self.transfers_task.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        let api = self.api.clone();
        self.transfers_task = Some(tokio::spawn(async move { api.downloads().await }));
    }

    /// Kick off a search without blocking the interface.
    pub fn start_search(&mut self) {
        if self.query.trim().is_empty() {
            return;
        }
        if let Some(h) = self.search_task.take() {
            h.abort();
        }
        self.tab = Tab::Search;
        self.results.clear();
        self.search_selected = 0;
        self.search_started = Some(Instant::now());
        self.status = format!("searching {:?} — Esc to cancel", self.query);

        let api = self.api.clone();
        let query = self.query.clone();
        self.search_task = Some(tokio::spawn(async move {
            api.search(&query, Duration::from_secs(90)).await
        }));
    }

    pub fn cancel_search(&mut self) {
        if let Some(h) = self.search_task.take() {
            h.abort();
            self.search_started = None;
            self.status = "search cancelled".into();
        }
    }

    /// Collect anything that finished since the last frame. Never blocks.
    pub async fn poll_tasks(&mut self) {
        if self.search_task.as_ref().is_some_and(|h| h.is_finished()) {
            let handle = self.search_task.take().expect("checked above");
            self.search_started = None;
            match handle.await {
                Ok(Ok(s)) => {
                    self.results = rank::rank(&s.responses, &self.filter());
                    self.search_selected = 0;
                    self.status = format!(
                        "{} matching files from {} responses ({} locked, filtered out)",
                        self.results.len(),
                        s.response_count,
                        s.locked_file_count
                    );
                }
                Ok(Err(e)) => self.status = format!("search failed: {e}"),
                Err(e) if e.is_cancelled() => {}
                Err(e) => self.status = format!("search task failed: {e}"),
            }
        }

        if self.transfers_task.as_ref().is_some_and(|h| h.is_finished()) {
            let handle = self.transfers_task.take().expect("checked above");
            match handle.await {
                Ok(Ok(users)) => {
                    self.transfers = flatten_transfers(&users);
                    self.reindex_transfers();
                    if self.transfer_selected >= self.transfers.len() {
                        self.transfer_selected = self.transfers.len().saturating_sub(1);
                    }
                }
                Ok(Err(e)) => self.status = format!("transfers failed: {e}"),
                Err(_) => {}
            }
        }
    }

    #[cfg(test)]
    pub fn reindex_for_test(&mut self) {
        self.reindex_transfers();
    }

    fn reindex_transfers(&mut self) {
        self.transfer_by_file = self
            .transfers
            .iter()
            .map(|t| ((t.username.clone(), t.filename.clone()), t.clone()))
            .collect();
        self.have_basenames = self
            .transfers
            .iter()
            .filter(|t| t.succeeded())
            .map(|t| crate::models::basename(&t.filename).to_lowercase())
            .collect();
    }

    /// What, if anything, has happened to this exact file from this exact peer.
    pub fn transfer_for(&self, c: &Candidate) -> Option<&Transfer> {
        self.transfer_by_file
            .get(&(c.username.clone(), c.file.filename.clone()))
    }

    /// Whether a file by this name already came down from somebody. Guards
    /// against queueing the same song twice from different peers.
    pub fn already_have(&self, c: &Candidate) -> bool {
        self.have_basenames
            .contains(&c.name().to_lowercase())
    }

    pub async fn activate(&mut self) {
        match self.tab {
            Tab::Search => self.enqueue_selected().await,
            Tab::Browse => {}
            Tab::Transfers => {}
        }
    }

    pub async fn enqueue_selected(&mut self) {
        let Some(c) = self.results.get(self.search_selected).cloned() else {
            return;
        };
        match self
            .api
            .enqueue(&c.username, std::slice::from_ref(&c.file))
            .await
        {
            Ok(()) => {
                self.status = format!("queued {} from {}", c.name(), c.username);
                // Show it as queued straight away. The next poll replaces this
                // with slskd's own view a second or two later, but the row must
                // react to the keypress now, not eventually.
                self.transfer_by_file.insert(
                    (c.username.clone(), c.file.filename.clone()),
                    Transfer {
                        username: c.username.clone(),
                        filename: c.file.filename.clone(),
                        state: "Queued, Locally".into(),
                        size: c.file.size,
                        ..Default::default()
                    },
                );
                self.refresh_transfers();
            }
            Err(e) => self.status = format!("queue failed: {e}"),
        }
    }

    pub async fn browse_selected(&mut self) {
        let user = match self.tab {
            Tab::Search => self.results.get(self.search_selected).map(|c| c.username.clone()),
            Tab::Transfers => self
                .transfers
                .get(self.transfer_selected)
                .map(|t| t.username.clone()),
            Tab::Browse => None,
        };
        let Some(user) = user else { return };

        self.status = format!("browsing {user}…");
        match self.api.browse(&user).await {
            Ok(r) => {
                self.browse_user = user;
                self.browse_dirs = r.directories;
                self.browse_selected = 0;
                self.tab = Tab::Browse;
                self.status = format!("{} directories", self.browse_dirs.len());
            }
            Err(e) => self.status = format!("browse failed: {e}"),
        }
    }

    pub async fn clear_completed(&mut self) {
        let finished = self.transfers.iter().filter(|t| t.is_done()).count();
        match self.api.clear_completed().await {
            Ok(()) => {
                self.status = format!("cleared {finished} finished transfers (done and failed)");
                self.refresh_transfers();
            }
            Err(e) => self.status = format!("clear failed: {e}"),
        }
    }

    /// Drop the currently selected transfer, finished or not.
    pub async fn remove_selected_transfer(&mut self) {
        let Some(t) = self.transfers.get(self.transfer_selected).cloned() else {
            return;
        };
        match self.api.remove_download(&t.username, &t.id).await {
            Ok(()) => {
                self.status = format!("removed {}", t.name());
                self.refresh_transfers();
            }
            Err(e) => self.status = format!("remove failed: {e}"),
        }
    }

    pub fn clear_browse(&mut self) {
        self.browse_dirs.clear();
        self.browse_user.clear();
        self.browse_selected = 0;
        self.status = "cleared browse listing".into();
    }
}
