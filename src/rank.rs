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
    /// Lowercased tokens that must all appear in the file's title field.
    pub title_tokens: Vec<String>,
    /// The full normalised title. Enforced as a contiguous phrase when
    /// `title_tokens` is too weak to identify a song on its own.
    pub title_phrase: Option<String>,
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
            title_phrase: None,
            artist_tokens: Vec::new(),
            extensions: Vec::new(),
            min_bitrate: 0,
            check_duration: true,
            reject_variants: true,
        }
    }
}

/// Fold a Latin letter with a diacritic down to its ASCII base.
///
/// Tokenising splits on anything non-ASCII-alphanumeric, so without this
/// "Águas" becomes the token `guas` and can never match a search for "Aguas" —
/// which is most of a Brazilian, French or Spanish library. Peers are
/// inconsistent about accents in filenames, so both sides get folded and the
/// comparison stops caring.
fn fold_char(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        'ý' | 'ÿ' => 'y',
        'š' => 's',
        'ž' => 'z',
        other => other,
    }
}

fn fold(s: &str) -> String {
    s.to_lowercase().chars().map(fold_char).collect()
}

/// Split a phrase into matchable tokens, dropping punctuation and noise words.
///
/// One-and-two-character tokens are dropped: they are mostly `a`, `of`, and the
/// wreckage of apostrophes (`don't` → `don`, `t`), and requiring them to appear
/// rejects correctly-named files over trivial punctuation differences.
pub fn tokenize(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(|t| t.to_string())
        .collect()
}

/// The title portion of a filename.
///
/// Peers commonly name files `Artist - Album - NN - Title.ext`, which means the
/// album name is sitting right there in the string the title is matched
/// against. Searching for the Doobie Brothers' "Minute By Minute" matched
/// `The Doobie Brothers - Minute by Minute - 01 - Here to Love You.mp3` — the
/// right album, the wrong song. Matching only against the final segment fixes
/// that, and degrades to the whole name when there are no separators.
///
/// Underscores are normalised first; `01_-_Artist_-_Title.mp3` is common.
fn title_field(basename: &str) -> String {
    let norm = basename.replace('_', " ");
    let stem = norm.rsplit_once('.').map(|(s, _)| s).unwrap_or(&norm).to_string();
    match stem.rsplit(" - ").next() {
        Some(last) if !last.trim().is_empty() => fold(last.trim()),
        _ => fold(&stem),
    }
}

/// Titles made almost entirely of short words survive tokenisation as one or
/// two weak tokens — "This Is It" becomes just `this`, which matches "This Is
/// How My Song Goes". When a title is that weak, require the whole phrase.
///
/// Strength is measured over *distinct* tokens. A repeated word carries no
/// extra matching power: "More More More" tokenises to three copies of `more`,
/// and requiring every one of them is satisfied by a single occurrence. Counted
/// naively it looked like a strong three-token title and skipped the phrase
/// check, which is how a search for it returned "More Than You'll Ever Know".
fn is_weak(title_tokens: &[String]) -> bool {
    let mut distinct: Vec<&String> = title_tokens.iter().collect();
    distinct.sort();
    distinct.dedup();
    distinct.len() < 2 || distinct.iter().map(|t| t.len()).sum::<usize>() < 8
}

pub fn normalize_phrase(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in fold(s).chars() {
        if c.is_ascii_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        } else {
            space = true;
        }
    }
    out
}

/// Whether a path plausibly names this artist.
///
/// A plain substring test is too eager for short names: searching Gal Costa
/// matched a Japanese single because "gal" appears inside "SINGALONG". A plain
/// token test is too strict the other way, because peers write names solid —
/// "BadenPowell" is one token and would never equal "baden".
///
/// So: short tokens must be a whole token in the path, longer ones may appear
/// inside one. Four characters is where accidental collisions stop being
/// common in practice.
fn path_mentions(path: &str, artist_token: &str) -> bool {
    // Must tokenize the path the same way the artist name was tokenized.
    // Using the stricter `tokenize` here dropped two-character tokens from the
    // path, so the "42" kept in "Level 42" could never be found and the band
    // became unmatchable.
    let tokens = tokenize_artist(path);
    if artist_token.len() >= 4 {
        tokens.iter().any(|p| p.contains(artist_token))
    } else {
        tokens.iter().any(|p| p == artist_token)
    }
}

/// Words that appear in artist names but identify nobody.
///
/// "Kool and the Gang" matched a Jimi Hendrix track because the album folder
/// contained the word "the". Grammar is not evidence.
const ARTIST_STOPWORDS: &[&str] = &[
    "the", "and", "of", "feat", "ft", "with", "his", "her", "band", "los", "las",
];

/// Tokenize an artist name.
///
/// Differs from [`tokenize`] in two ways: short numeric tokens are kept, because
/// the number in "Level 42" is the only thing separating it from any other band
/// with "Level" in the name; and stopwords are dropped.
pub fn tokenize_artist(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| {
            (t.len() > 2 || (t.len() == 2 && t.chars().all(|c| c.is_ascii_digit())))
                && !ARTIST_STOPWORDS.contains(t)
        })
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
    "acapella", "acoustic", "reprise", "demo", "rehearsal", "megamix", "edit",
    // Re-cut / re-production markers used by the edit scene.
    "revibe", "rework", "redux", "reedit", "refix", "rerub", "dub",
    // DJ-pool markers. These files are cut for mixing, not listening.
    "intro", "clean", "dirty", "acap", "quickie", "transition",
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

        // Match the title against the title field, not the whole filename, so
        // an album name embedded in the path cannot stand in for the song.
        let field = title_field(c.name());
        if !self.title_tokens.iter().all(|t| field.contains(t.as_str())) {
            return false;
        }
        if let Some(phrase) = &self.title_phrase {
            if is_weak(&self.title_tokens) && !normalize_phrase(&field).contains(phrase.as_str()) {
                return false;
            }
        }

        if !self.artist_tokens.is_empty() {
            // Require two independent pieces of evidence when the name offers
            // them. One token is too easy to hit by accident: "Level 42"
            // reduced to "level" matched a band called Red Level. Names with a
            // single distinctive token still only need the one.
            let needed = self.artist_tokens.len().min(2);
            let found = self
                .artist_tokens
                .iter()
                .filter(|t| path_mentions(&c.file.filename, t))
                .count();
            if found < needed {
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

            // Some markers get glued to neighbouring words -- "RemixPack",
            // "BootlegMix" -- and survive tokenising intact. For the
            // unambiguous ones, a substring anywhere in the name is enough.
            const ALWAYS_SUBSTRING: &[&str] = &["remix", "karaoke", "bootleg", "megamix"];
            let folded_name = fold(c.name());
            for v in ALWAYS_SUBSTRING {
                if folded_name.contains(v) && !self.title_tokens.iter().any(|t| t == v) {
                    return false;
                }
            }

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
    fn album_name_in_the_filename_cannot_stand_in_for_the_title() {
        // Real miss: searching the Doobie Brothers' "Minute By Minute" matched
        // a file from that album whose actual song is something else. The album
        // name sits in the filename, so a naive substring test passes.
        let mut f0 = file("x.mp3", Some(320), Some(280));
        f0.filename =
            "@@peer\\Music\\Minute by Minute\\The Doobie Brothers - Minute by Minute - 01 - Here to Love You.mp3"
                .to_string();
        let f = Filter {
            title_tokens: tokenize("Minute By Minute"),
            title_phrase: Some(normalize_phrase("Minute By Minute")),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&[resp("peer", true, 0, 100, vec![f0])], &f).is_empty());
    }

    #[test]
    fn a_title_of_short_words_must_match_as_a_phrase() {
        // Real miss: "This Is It" tokenises to just ["this"], which matched
        // "This Is How My Song Goes" from the same artist.
        let mut wrong = file("x.mp3", Some(320), Some(240));
        wrong.filename =
            "@@peer\\Music\\It's About Time\\Kenny Loggins - It's About Time - 07 - This Is How My Song Goes.mp3"
                .to_string();
        let mut right = file("y.mp3", Some(320), Some(238));
        right.filename = "@@peer\\Music\\Keep the Fire\\Kenny Loggins - This Is It.mp3".to_string();

        let f = Filter {
            title_tokens: tokenize("This Is It"),
            title_phrase: Some(normalize_phrase("This Is It")),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        let ranked = rank(&[resp("peer", true, 0, 100, vec![wrong, right])], &f);
        assert_eq!(ranked.len(), 1, "only the real track should survive");
        assert!(ranked[0].file.filename.ends_with("This Is It.mp3"));
    }

    #[test]
    fn title_field_handles_the_common_naming_shapes() {
        assert_eq!(title_field("01 Steal Away.mp3"), "01 steal away");
        assert_eq!(title_field("Artist - Title.mp3"), "title");
        assert_eq!(title_field("A - B - 07 - Real Title.mp3"), "real title");
        // underscore-separated names are common from some clients
        assert_eq!(title_field("02_-_Hall_&_Oates_-_Rich_Girl.mp3"), "rich girl");
    }

    #[test]
    fn a_repeated_word_does_not_count_as_a_strong_title() {
        // Real miss: "More More More" looked like three tokens, skipped the
        // phrase check, and matched "More Than You'll Ever Know".
        let mut wrong = file("x.mp3", Some(320), Some(230));
        wrong.filename =
            "@@peer\\Music\\Greatest Hits\\14 - Milli Vanilli - More Than You'll Ever Know.mp3"
                .to_string();
        let mut right = file("y.mp3", Some(320), Some(215));
        right.filename =
            "@@peer\\Music\\Disco\\Andrea True Connection - More More More.mp3".to_string();

        let f = Filter {
            title_tokens: tokenize("More More More"),
            title_phrase: Some(normalize_phrase("More More More")),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        let ranked = rank(&[resp("peer", true, 0, 100, vec![wrong, right])], &f);
        assert_eq!(ranked.len(), 1, "only the real track should survive");
        assert!(ranked[0].file.filename.contains("More More More"));
    }

    #[test]
    fn a_strong_title_still_matches_without_the_phrase_being_contiguous() {
        // "Laughter In The Rain" is strong enough to match on tokens alone,
        // which matters because peers reorder and punctuate freely.
        let f = Filter {
            title_tokens: tokenize("Laughter In The Rain"),
            title_phrase: Some(normalize_phrase("Laughter In The Rain")),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        let g = file("086. Neil Sedaka - Laughter In The Rain.mp3", Some(320), Some(170));
        assert_eq!(rank(&[resp("peer", true, 0, 100, vec![g])], &f).len(), 1);
    }

    #[test]
    fn accents_fold_so_unaccented_queries_still_match() {
        // Peers spell Brazilian titles both ways. Without folding, "Águas"
        // tokenises to ["guas"] and a search for "Aguas de Marco" finds nothing.
        assert_eq!(tokenize("Águas de Março"), ["aguas", "marco"]);
        assert_eq!(tokenize("Começar de Novo"), ["comecar", "novo"]);
        assert_eq!(normalize_phrase("Manhã de Carnaval"), "manha de carnaval");

        let g = file("01 - Elis Regina - Águas de Março.mp3", Some(320), Some(215));
        let f = Filter {
            title_tokens: tokenize("Aguas de Marco"),
            title_phrase: Some(normalize_phrase("Aguas de Marco")),
            artist_tokens: tokenize("Elis Regina"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert_eq!(rank(&[resp("peer", true, 0, 100, vec![g])], &f).len(), 1);
    }

    #[test]
    fn accented_query_matches_an_unaccented_file() {
        let g = file("Marcos Valle - Nao Tem Nada Nao.mp3", Some(320), Some(200));
        let f = Filter {
            title_tokens: tokenize("Não Tem Nada Não"),
            title_phrase: Some(normalize_phrase("Não Tem Nada Não")),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert_eq!(rank(&[resp("peer", true, 0, 100, vec![g])], &f).len(), 1);
    }

    #[test]
    fn a_short_artist_token_must_be_a_whole_word() {
        // Real miss: searching Gal Costa's "Baby" queued a Japanese single,
        // because "gal" sits inside "SINGALONG".
        let mut wrong = file("06. Shout Baby.mp3", Some(320), Some(240));
        wrong.filename =
            "@@peer\\Music\\SINGALONG (2020) [WEB 320]\\06. Shout Baby.mp3".to_string();
        let f = Filter {
            title_tokens: tokenize("Baby"),
            title_phrase: Some(normalize_phrase("Baby")),
            artist_tokens: tokenize("Gal Costa"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&[resp("peer", true, 0, 100, vec![wrong])], &f).is_empty());
    }

    #[test]
    fn a_longer_artist_token_still_matches_a_run_together_name() {
        // Peers write names solid: "BadenPowell" is a single token.
        let mut g = file("x.mp3", Some(320), Some(200));
        g.filename =
            "@@peer\\VA - Guitar Workshop\\6_BadenPowell_Berimbau.mp3".to_string();
        let f = Filter {
            title_tokens: tokenize("Berimbau"),
            title_phrase: Some(normalize_phrase("Berimbau")),
            artist_tokens: tokenize("Baden Powell"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert_eq!(rank(&[resp("peer", true, 0, 100, vec![g])], &f).len(), 1);
    }

    #[test]
    fn an_edit_is_rejected_like_any_other_variant() {
        let g = file("Quarteto Em Cy - Tudo Que Voce Podia Ser (Querelas Do Brazil Edit).mp3",
                     Some(320), Some(250));
        let f = Filter {
            title_tokens: tokenize("Tudo Que Voce Podia Ser"),
            title_phrase: Some(normalize_phrase("Tudo Que Voce Podia Ser")),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&[resp("peer", true, 0, 100, vec![g])], &f).is_empty());
    }

    #[test]
    fn a_number_in_the_band_name_is_kept() {
        // Real miss: "Level 42" reduced to ["level"] and matched an emo band
        // called Red Level.
        assert_eq!(tokenize_artist("Level 42"), ["level", "42"]);
        let mut wrong = file("10.Turn It On.mp3", Some(320), Some(200));
        wrong.filename =
            "@@p\\VA - Emo Diaries No. 1\\Red Level - 10.Turn It On.mp3".to_string();
        let f = Filter {
            title_tokens: tokenize("Turn It On"),
            title_phrase: Some(normalize_phrase("Turn It On")),
            artist_tokens: tokenize_artist("Level 42"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&[resp("p", true, 0, 100, vec![wrong])], &f).is_empty());
    }

    #[test]
    fn grammar_words_are_not_evidence_of_an_artist() {
        // Real miss: "Kool and the Gang" matched a Hendrix album folder
        // containing the word "the".
        assert_eq!(tokenize_artist("Kool and the Gang"), ["kool", "gang"]);
        let mut wrong = file("12 - Straight Ahead.mp3", Some(320), Some(280));
        wrong.filename =
            "@@p\\First Rays of the New Rising Sun\\12 - Straight Ahead.mp3".to_string();
        let f = Filter {
            title_tokens: tokenize("Straight Ahead"),
            title_phrase: Some(normalize_phrase("Straight Ahead")),
            artist_tokens: tokenize_artist("Kool and the Gang"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&[resp("p", true, 0, 100, vec![wrong])], &f).is_empty());
    }

    #[test]
    fn a_marker_glued_to_another_word_still_counts() {
        // "RemixPack" survives tokenising as one token and is not "remix".
        let g = file("Shalamar - A Night To Remember (Cesar Vilo RemixPack 4).mp3",
                     Some(320), Some(240));
        let f = Filter {
            title_tokens: tokenize("A Night To Remember"),
            title_phrase: Some(normalize_phrase("A Night To Remember")),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&[resp("p", true, 0, 100, vec![g])], &f).is_empty());
    }

    #[test]
    fn a_kept_number_can_actually_be_found_in_the_path() {
        // The query keeps "42"; the path tokenizer has to keep it too.
        let mut g = file("x.mp3", Some(320), Some(221));
        g.filename = "@@p\\Turn It On\\Level 42_Turn It On_08_Turn It On.mp3".to_string();
        let f = Filter {
            title_tokens: tokenize("Turn It On"),
            title_phrase: Some(normalize_phrase("Turn It On")),
            artist_tokens: tokenize_artist("Level 42"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert_eq!(rank(&[resp("p", true, 0, 100, vec![g])], &f).len(), 1);
    }

    #[test]
    fn a_dj_pool_intro_cut_is_rejected() {
        // "(Sickmix Intro) (Clean)" is cut for mixing. "sickmix" is not the
        // token "mix", so the marker that catches it is "intro".
        let g = file("Turn It On Level 42 (Sickmix Intro) (Clean) 115.mp3", Some(320), Some(300));
        let f = Filter {
            title_tokens: tokenize("Turn It On"),
            title_phrase: Some(normalize_phrase("Turn It On")),
            artist_tokens: tokenize_artist("Level 42"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&[resp("p", true, 0, 100, vec![g])], &f).is_empty());
    }

    #[test]
    fn one_distinctive_artist_token_is_still_enough() {
        // Single-word names must not be made impossible by the two-token rule.
        let g = file("Skyy - Call Me.mp3", Some(320), Some(230));
        let f = Filter {
            title_tokens: tokenize("Call Me"),
            title_phrase: Some(normalize_phrase("Call Me")),
            artist_tokens: tokenize_artist("Skyy"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert_eq!(rank(&[resp("p", true, 0, 100, vec![g])], &f).len(), 1);
    }

    #[test]
    fn an_edit_scene_rework_is_rejected() {
        // "(Dario Caminita Revibe)" is a re-production, not the 1976 single.
        let g = file("KC & THE SUNSHINE BAND - I'm Your Boogie Man (Dario Caminita Revibe).mp3",
                     Some(320), Some(300));
        let f = Filter {
            title_tokens: tokenize("Im Your Boogie Man"),
            title_phrase: Some(normalize_phrase("Im Your Boogie Man")),
            artist_tokens: tokenize_artist("KC Sunshine"),
            extensions: vec!["mp3".into()],
            ..Default::default()
        };
        assert!(rank(&[resp("p", true, 0, 100, vec![g])], &f).is_empty());
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
