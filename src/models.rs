//! Wire types for the slskd v0 API.
//!
//! Only the fields this client actually uses are modelled; slskd returns a good
//! deal more. Every struct is `#[serde(default)]` so a slskd upgrade that adds or
//! drops a field degrades gracefully instead of failing the whole response.

use serde::{Deserialize, Serialize};

/// Soulseek paths are Windows-style, backslash separated, regardless of the
/// platform on either end. Everything user-facing goes through here.
pub fn basename(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

/// The containing folder, which for most shares is the album.
pub fn parent(path: &str) -> &str {
    match path.rfind('\\') {
        Some(i) => {
            let dir = &path[..i];
            dir.rsplit('\\').next().unwrap_or(dir)
        }
        None => "",
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Application {
    pub version: Version,
    pub server: Server,
    pub user: UserState,
    pub shares: Shares,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Version {
    pub current: String,
    pub latest: String,
    pub is_update_available: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Server {
    pub address: String,
    pub state: String,
    pub is_connected: bool,
    pub is_logged_in: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct UserState {
    pub username: String,
    pub statistics: UserStatistics,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct UserStatistics {
    pub file_count: i64,
    pub directory_count: i64,
    pub average_speed: i64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Shares {
    pub ready: bool,
    pub scanning: bool,
    pub files: i64,
    pub directories: i64,
}

// ---------------------------------------------------------------- searches

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub id: String,
    pub search_text: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Search {
    pub id: String,
    pub search_text: String,
    /// `InProgress`, then a terminal state. Note that `"Completed, TimedOut"` is
    /// the *normal* ending for a search that ran its clock out with results.
    pub state: String,
    pub file_count: i64,
    pub locked_file_count: i64,
    pub response_count: i64,
    pub responses: Vec<SearchResponse>,
}

impl Search {
    pub fn in_progress(&self) -> bool {
        self.state == "InProgress"
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchResponse {
    pub username: String,
    pub has_free_upload_slot: bool,
    pub queue_length: i64,
    pub upload_speed: i64,
    pub file_count: i64,
    pub files: Vec<File>,
    /// Files the peer will not serve. Kept separate so they are never queued.
    pub locked_files: Vec<File>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct File {
    pub filename: String,
    pub size: i64,
    pub bit_rate: Option<i64>,
    /// Duration in seconds. Absent for lossless and for peers that don't scan.
    pub length: Option<i64>,
    pub is_locked: bool,
}

/// One queueable file plus the peer it came from.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub username: String,
    pub has_free_slot: bool,
    pub queue_length: i64,
    pub upload_speed: i64,
    pub file: File,
}

impl Candidate {
    pub fn name(&self) -> &str {
        basename(&self.file.filename)
    }
    pub fn album(&self) -> &str {
        parent(&self.file.filename)
    }
    pub fn extension(&self) -> String {
        basename(&self.file.filename)
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueRequest {
    pub filename: String,
    pub size: i64,
}

// --------------------------------------------------------------- transfers

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TransferUser {
    pub username: String,
    pub directories: Vec<TransferDirectory>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TransferDirectory {
    pub directory: String,
    pub files: Vec<Transfer>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Transfer {
    pub id: String,
    pub username: String,
    pub filename: String,
    pub state: String,
    pub size: i64,
    pub bytes_transferred: i64,
    pub percent_complete: f64,
    pub average_speed: f64,
    pub exception: Option<String>,
    /// When the download was asked for. Used to show newest first; slskd
    /// returns transfers grouped by peer, which is close to arbitrary order.
    pub requested_at: Option<String>,
    pub enqueued_at: Option<String>,
}

impl Transfer {
    pub fn name(&self) -> &str {
        basename(&self.filename)
    }
    pub fn is_done(&self) -> bool {
        self.state.starts_with("Completed")
    }
    pub fn succeeded(&self) -> bool {
        self.state == "Completed, Succeeded"
    }
    pub fn errored(&self) -> bool {
        self.state.starts_with("Completed") && !self.succeeded()
    }
    /// Sort key for "most recent first". ISO-8601 timestamps sort correctly as
    /// strings, so no date parsing is needed just to order a list.
    pub fn sort_key(&self) -> &str {
        self.requested_at
            .as_deref()
            .or(self.enqueued_at.as_deref())
            .unwrap_or("")
    }
}

/// Flatten slskd's user → directory → file nesting into a plain list, most
/// recently requested first.
///
/// slskd groups transfers by peer, so the raw order tracks whichever peer
/// happens to be listed first rather than anything the user did. Newest-first
/// puts what you just queued where you are already looking.
pub fn flatten(users: &[TransferUser]) -> Vec<Transfer> {
    let mut out: Vec<Transfer> = users
        .iter()
        .flat_map(|u| u.directories.iter())
        .flat_map(|d| d.files.iter())
        .cloned()
        .collect();
    out.sort_by(|a, b| b.sort_key().cmp(a.sort_key()));
    out
}

// ------------------------------------------------------------------ browse

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowseResult {
    pub directories: Vec<BrowseDirectory>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowseDirectory {
    pub name: String,
    pub file_count: i64,
    pub files: Vec<File>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct UserInfo {
    pub description: String,
    pub upload_slots: i64,
    pub queue_length: i64,
    pub has_free_upload_slot: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct UserStatus {
    pub presence: String,
    pub is_privileged: bool,
}

#[cfg(test)]
mod order_tests {
    use super::*;

    fn t(name: &str, requested: &str) -> Transfer {
        Transfer {
            filename: format!("@@p\\Music\\{name}"),
            requested_at: Some(requested.into()),
            ..Default::default()
        }
    }

    #[test]
    fn flatten_puts_the_newest_request_first() {
        // slskd groups by peer, so raw order follows whichever peer is listed
        // first. What you just queued must appear at the top.
        let users = vec![
            TransferUser {
                username: "old_peer".into(),
                directories: vec![TransferDirectory {
                    directory: "d".into(),
                    files: vec![t("first.mp3", "2026-08-24T05:00:00")],
                }],
            },
            TransferUser {
                username: "new_peer".into(),
                directories: vec![TransferDirectory {
                    directory: "d".into(),
                    files: vec![t("latest.mp3", "2026-08-24T09:30:00")],
                }],
            },
        ];
        let out = flatten(&users);
        assert_eq!(out[0].name(), "latest.mp3");
        assert_eq!(out[1].name(), "first.mp3");
    }

    #[test]
    fn transfers_without_timestamps_do_not_panic_the_sort() {
        let users = vec![TransferUser {
            username: "p".into(),
            directories: vec![TransferDirectory {
                directory: "d".into(),
                files: vec![
                    Transfer { filename: "a.mp3".into(), ..Default::default() },
                    t("b.mp3", "2026-08-24T05:00:00"),
                ],
            }],
        }];
        let out = flatten(&users);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name(), "b.mp3", "timestamped entries sort ahead of unknown");
    }
}
