//! Command-line surface.

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "slsk",
    version,
    about = "A terminal client for slskd",
    long_about = "Search Soulseek, queue downloads, and watch transfers from the terminal.\n\
                  Run with no subcommand to open the interactive TUI."
)]
pub struct Cli {
    /// Override the endpoint instead of auto-detecting from config.
    #[arg(long, global = true, value_name = "URL")]
    pub url: Option<String>,

    /// Print the resolved endpoint before running.
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Write a starter config file.
    Init,

    /// Show server connection, shares, and version.
    Status,

    /// Search and print ranked results without downloading.
    Search(SearchArgs),

    /// Search, pick the best source, and queue it.
    Get(GetArgs),

    /// List current transfers.
    Transfers(TransferArgs),

    /// Browse another user's shared files.
    Browse(BrowseArgs),
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// What to search for.
    pub query: Vec<String>,

    /// Require this artist to appear in the path.
    #[arg(long)]
    pub artist: Option<String>,

    /// Restrict to an extension. Repeatable. Defaults to mp3.
    #[arg(long = "ext", value_name = "EXT")]
    pub extensions: Vec<String>,

    /// Allow any audio format, not just mp3.
    #[arg(long)]
    pub any_format: bool,

    /// Minimum bitrate in kbps.
    #[arg(long, default_value_t = 0)]
    pub min_bitrate: i64,

    /// Skip the duration sanity check that filters edits and DJ mixes.
    #[arg(long)]
    pub no_duration_check: bool,

    /// Keep remixes, covers, karaoke and live cuts in the results.
    #[arg(long)]
    pub allow_variants: bool,

    /// How many results to show.
    #[arg(long, short = 'n', default_value_t = 15)]
    pub limit: usize,

    /// Seconds to let the search run.
    #[arg(long, default_value_t = 30)]
    pub wait: u64,
}

#[derive(Args, Debug)]
pub struct GetArgs {
    #[command(flatten)]
    pub search: SearchArgs,

    /// Queue without asking for confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Queue the top N sources instead of just the best one.
    #[arg(long, default_value_t = 1)]
    pub count: usize,
}

#[derive(Args, Debug)]
pub struct TransferArgs {
    /// Refresh until every transfer reaches a terminal state.
    #[arg(long, short = 'w')]
    pub watch: bool,

    /// Remove completed and errored records, then exit.
    #[arg(long)]
    pub clear: bool,

    /// Show uploads instead of downloads.
    #[arg(long)]
    pub uploads: bool,
}

#[derive(Args, Debug)]
pub struct BrowseArgs {
    /// Username to browse.
    pub username: String,

    /// Only show directories whose name contains this.
    #[arg(long)]
    pub filter: Option<String>,

    /// List files inside matching directories.
    #[arg(long, short = 'f')]
    pub files: bool,

    /// Maximum directories to print.
    #[arg(long, short = 'n', default_value_t = 40)]
    pub limit: usize,
}

impl SearchArgs {
    pub fn query_string(&self) -> String {
        self.query.join(" ")
    }

    pub fn to_filter(&self) -> crate::rank::Filter {
        let extensions = if self.any_format {
            Vec::new()
        } else if self.extensions.is_empty() {
            vec!["mp3".to_string()]
        } else {
            self.extensions.iter().map(|e| e.to_lowercase()).collect()
        };

        crate::rank::Filter {
            title_tokens: crate::rank::tokenize(&self.query_string()),
            artist_tokens: self
                .artist
                .as_deref()
                .map(crate::rank::tokenize)
                .unwrap_or_default(),
            extensions,
            min_bitrate: self.min_bitrate,
            check_duration: !self.no_duration_check,
            reject_variants: !self.allow_variants,
        }
    }
}
