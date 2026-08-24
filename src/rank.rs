//! Choosing which copy of a file to actually download.
//!
//! A Soulseek search for a well-known song returns thousands of files from
//! hundreds of peers. They are not interchangeable: most of the difference
//! between a good grab and a bad one is picking the right source, not the right
//! search terms. The rules encoded here, in priority order:
//!
//! 1. **A free upload slot.** Without one you join a queue that may never move.
//! 2. **A short queue.** Peers advertising 300+ queued files are effectively
//!    offline for casual downloads.
//! 3. **Plausible duration.** This is the one that catches real mistakes. A
//!    search for a 3:26 album track will happily return a 1:30 radio edit, a
//!    30-second sample, or a 74-minute DJ mix that merely mentions it. Filtering
//!    on duration removes a class of wrong result that bitrate alone never will.
//! 4. **Bitrate**, capped — 320 and 256 are both fine, and above 320 usually
//!    means a transcode or a mislabel rather than a better file.
//! 5. **Upload speed**, last. It only decides how fast a already-good file lands.

use crate::models::{Candidate, SearchResponse};

/// Duration bounds, in seconds, for "this is plausibly the song I asked for".
/// Generous on both ends: short soul singles run under two minutes, and album
/// versions of soft-rock tracks can pass seven.
pub const MIN_LEN: i64 = 100;
pub const MAX_LEN: i64 = 600;

#[derive(Debug, Clone)]
pub struct Filter {
    /// Lowercased tokens that must all appear in the file's basename.
    pub title_tokens: Vec<String>,
    /// Lowercased tokens, at least one of which must appear in the full path.
    /// Empty means no artist constraint.
    pub artist_tokens: Vec<String>,
    /// Restrict to these lowercase extensions. Empty means allow any audio.
    pub extensions: Vec<String>,
    pub min_bitrate: i64,
    pub check_duration: bool,
    /// Reject remixes, covers, karaoke and live cuts unless the query asked for
    /// one. See [`VARIANT_TOKENS`].
    pub reject_variants: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            title_tokens: Vec::new(),
            artist_tokens: Vec::new(),
            extensions: Vec::new(),
            min_bitrate: 0,
            check_duration: true,
            reject_variants: true,
        }
    }
}

/// Split a phrase into matchable tokens, dropping punctuation and noise words.
///
/// One-and-two-character tokens are dropped: they are mostly `a`, `of`, and the
/// wreckage of apostrophes (`don't` → `don`, `t`), and requiring them to appear
/// rejects correctly-named files over trivial punctuation differences.
pub fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(|t| t.to_string())
        .collect()
}

const AUDIO: &[&str] = &["mp3", "flac", "m4a", "ogg", "oga", "opus", "wav", "wma"];

/// Tokens that mean "this is not the recording you asked for".
///
/// Duration filtering catches short edits and long mixes, but it cannot catch a
/// full-length remix or a faithful cover — those have entirely plausible
/// runtimes. These are matched as whole tokens, never substrings, so `mix`
/// rejects `(Ken@Work Mix)` without touching a band called Mixtapes.
///
/// A token here is only disqualifying when the query did *not* ask for it: search
/// for "steely dan peg live" and live versions stay in the running.
const VARIANT_TOKENS: &[&str] = &[
    "remix", "rmx", "mix", "karaoke", "tribute", "cover", "instrumental",
    "acapella", "acoustic", "reprise", "demo", "rehearsal", "megamix",
];

/// Variant tokens judged against the *whole path* rather than the filename.
///
/// A compilation folder named "A Tribute To Daryl Hall And John Oates" is the
/// tell that every file inside it is a cover, even though the filenames
/// themselves look exactly like the originals.
const PATH_VARIANT_TOKENS: &[&str] = &["tribute", "karaoke", "covers"];

impl Filter {
    fn accepts(&self, c: &Candidate) -> bool {
        if c.file.is_locked {
            return false;
        }
        let ext = c.extension();
        if self.extensions.is_empty() {
            if !AUDIO.contains(&ext.as_str()) {
                return false;
            }
        } else if !self.extensions.contains(&ext) {
            return false;
        }

        let base = c.name().to_lowercase();
        if !self.title_tokens.iter().all(|t| base.contains(t.as_str())) {
            return false;
        }

        if !self.artist_tokens.is_empty() {
            let full = c.file.filename.to_lowercase();
            if !self.artist_tokens.iter().any(|t| full.contains(t.as_str())) {
                return false;
            }
        }

        if self.min_bitrate > 0 {
            // Lossless files report no bitrate; don't punish them for it.
            if let Some(br) = c.file.bit_rate {
                if br < self.min_bitrate {
                    return false;
                }
            }
        }

        if self.reject_variants {
            // Never reject on a token the caller explicitly searched for.
            let asked: Vec<&String> = self.title_tokens.iter().collect();

            let name_tokens = tokenize(c.name());
            for v in VARIANT_TOKENS {
                if name_tokens.iter().any(|t| t == v)
                    && !asked.iter().any(|t| t.as_str() == *v)
                {
                    return false;
                }
            }

            let path_tokens = tokenize(&c.file.filename);
            for v in PATH_VARIANT_TOKENS {
                if path_tokens.iter().any(|t| t == v)
                    && !asked.iter().any(|t| t.as_str() == *v)
                {
                    return false;
                }
            }
        }

        if self.check_duration {
            if let Some(len) = c.file.length {
                // Only judge a duration the peer actually reported.
                if len > 0 && !(MIN_LEN..=MAX_LEN).contains(&len) {
                    return false;
                }
            }
        }

        true
    }
}

/// Score used to order candidates. Higher is better; compared lexicographically.
fn score(c: &Candidate) -> (bool, bool, i64, i64) {
    (
        c.has_free_slot,
        c.queue_length == 0,
        c.file.bit_rate.unwrap_or(0).min(320),
        c.upload_speed,
    )
}

/// Flatten search responses into candidates, drop the unsuitable, best first.
pub fn rank(responses: &[SearchResponse], filter: &Filter) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = responses
        .iter()
        .flat_map(|r| {
            r.files.iter().map(move |f| Candidate {
                username: r.username.clone(),
                has_free_slot: r.has_free_upload_slot,
                queue_length: r.queue_length,
                upload_speed: r.upload_speed,
                file: f.clone(),
            })
        })
        .filter(|c| filter.accepts(c))
        .collect();

    out.sort_by(|a, b| score(b).cmp(&score(a)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::File;

    fn resp(user: &str, slot: bool, queue: i64, speed: i64, files: Vec<File>) -> SearchResponse {
        SearchResponse {
            username: user.into(),
            has_free_upload_slot: slot,
            queue_length: queue,
            upload_speed: speed,
            file_count: files.len() as i64,
            files,
            locked_files: vec![],
        }
    }

    fn file(name: &str, br: Option<i64>, len: Option<i64>) -> File {
        File {
            filename: format!("@@user\\Music\\Album\\{name}"),
            size: 8_000_000,
            bit_rate: br,
            length: len,
            is_locked: false,
        }
    }

    #[test]
    fn tokenize_drops_punctuation_and_short_words() {
        assert_eq!(tokenize("Alone Again (Naturally)"), ["alone", "again", "naturally"]);
        // "don" survives, "t" does not — matching "Don't" against "Dont" still works.
        assert_eq!(tokenize("What You Won't Do"), ["what", "you", "won"]);
    }

    #[test]
    fn rejects_a_radio_edit_by_duration() {
        // The exact failure this exists to prevent: a 2:35 edit outranking the
        // album cut purely because the peer serving it is faster.
        let responses = vec![
            resp("fastedit", true, 0, 9_000_000, vec![file("Steal Away.mp3", Some(320), Some(95))]),
            resp("slowfull", true, 0, 1_000_000, vec![file("Steal Away.mp3", Some(320), Some(206))]),
        ];
        let f = Filter {
            title_tokens: tokenize("Steal Away"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        let ranked = rank(&responses, &f);
        assert_eq!(ranked.len(), 1, "the 95-second edit should be filtered out");
        assert_eq!(ranked[0].username, "slowfull");
    }

    #[test]
    fn free_slot_beats_raw_speed() {
        let responses = vec![
            resp("busy", false, 40, 9_000_000, vec![file("Peg.mp3", Some(320), Some(237))]),
            resp("open", true, 0, 500_000, vec![file("Peg.mp3", Some(320), Some(237))]),
        ];
        let f = Filter {
            title_tokens: tokenize("Peg"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        let ranked = rank(&responses, &f);
        assert_eq!(ranked[0].username, "open");
    }

    #[test]
    fn bitrate_is_capped_so_absurd_values_do_not_win() {
        let responses = vec![
            resp("transcode", true, 0, 100, vec![file("Africa.mp3", Some(999), Some(295))]),
            resp("honest", true, 0, 200, vec![file("Africa.mp3", Some(320), Some(295))]),
        ];
        let f = Filter {
            title_tokens: tokenize("Africa"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        let ranked = rank(&responses, &f);
        assert_eq!(ranked[0].username, "honest", "999kbps must not outrank 320");
    }

    #[test]
    fn artist_token_filters_out_a_coincidental_title_match() {
        let responses = vec![resp(
            "wrong",
            true,
            0,
            100,
            vec![file("Dreams.mp3", Some(320), Some(250))],
        )];
        let f = Filter {
            title_tokens: tokenize("Dreams"),
            artist_tokens: tokenize("Fleetwood Mac"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&responses, &f).is_empty());
    }

    #[test]
    fn rejects_a_remix_of_the_right_song() {
        // Real miss: "THIS IS IT (STARTING FROM SCRATCH RMX)" outranked the
        // original because a remix has a perfectly plausible runtime.
        let responses = vec![
            resp("remixer", true, 0, 9_000_000,
                 vec![file("This Is It (Starting From Scratch RMX).mp3", Some(320), Some(240))]),
            resp("original", true, 0, 100,
                 vec![file("This Is It.mp3", Some(320), Some(238))]),
        ];
        let f = Filter {
            title_tokens: tokenize("This Is It"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        let ranked = rank(&responses, &f);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].username, "original");
    }

    #[test]
    fn rejects_a_cover_hiding_in_a_tribute_album() {
        // Real miss: searching Hall & Oates matched "A Tribute To Daryl Hall
        // And John Oates" and downloaded a Bird and the Bee cover. The filename
        // alone is innocent; only the folder gives it away.
        let mut f0 = file("Sara Smile.mp3", Some(320), Some(215));
        f0.filename =
            "@@peer\\Music\\Interpreting The Masters Volume 1 A Tribute To Daryl Hall And John Oates\\Sara Smile.mp3"
                .to_string();
        let responses = vec![resp("coverband", true, 0, 9_000_000, vec![f0])];
        let f = Filter {
            title_tokens: tokenize("Sara Smile"),
            artist_tokens: tokenize("Hall Oates"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&responses, &f).is_empty());
    }

    #[test]
    fn keeps_a_variant_the_query_actually_asked_for() {
        let responses = vec![resp(
            "dj", true, 0, 100,
            vec![file("Lowdown (Ken@Work Mix).mp3", Some(320), Some(300))],
        )];
        let f = Filter {
            title_tokens: tokenize("Lowdown Mix"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert_eq!(rank(&responses, &f).len(), 1, "asked for a mix, should get one");
    }

    #[test]
    fn variant_tokens_match_whole_words_only() {
        // "Mixtapes" must not trip the "mix" rule.
        let responses = vec![resp(
            "band", true, 0, 100,
            vec![file("Mixtapes And Cigarettes.mp3", Some(320), Some(200))],
        )];
        let f = Filter {
            title_tokens: tokenize("Mixtapes And Cigarettes"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert_eq!(rank(&responses, &f).len(), 1);
    }

    #[test]
    fn locked_files_are_never_candidates() {
        let mut f0 = file("Rosanna.mp3", Some(320), Some(311));
        f0.is_locked = true;
        let responses = vec![resp("locked", true, 0, 9_000_000, vec![f0])];
        let f = Filter {
            title_tokens: tokenize("Rosanna"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&responses, &f).is_empty());
    }
}
