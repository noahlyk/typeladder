//! Which words are practice words: the one predicate behind the pool, the seeding of `vocab.json`,
//! `typeladder vocab clean` and the adaptive feed.
//!
//! A word qualifies when all of these hold ([`WordFilter::accepts`]):
//!
//! 1. **Shape.** Letters only (an inner apostrophe only in a real contraction of [`CONTRACTIONS`]),
//!    two letters or more except `a` and `I`.
//! 2. **A real word.** One of: a contraction of the list; a word of Monkeytype's `english`,
//!    `english_1k`, `_5k`, `_10k` or `_25k` lists; a word of [`SPOKEN_WORDS`] (ordinary words the
//!    lists leave out: `ok`, the profanity of speech); a plain single word of one of the user's
//!    dictionaries or word lists. `--deep` adds any word of Monkeytype's `english_450k` dictionary.
//! 3. **Not a fragment.** Not a suffix, prefix or letter pair of [`EXTRA_FRAGMENTS`], unless it is
//!    also a real word (Monkeytype's lists up to the 10k, or [`REAL_WORDS`]: `age`, `less`, `over`).
//! 4. **Not noise.** Not a vocal noise or exclamation ([`noise_letters`]).
//!
//! `--deep` keeps 3 and 4. A capitalised word is a name and fails 1 unless `--names`.

use super::feed::is_plain_word;
use super::pool::{list_path, parse_monkeytype};
use std::collections::HashSet;
use std::path::Path;

/// Version of this filter: stored as `filter_version` in `vocab.json`; a file with a lower one is
/// cleaned once by `typeladder`. Bump it when the predicate changes what it accepts.
pub const FILTER_VERSION: u32 = 1;
/// Ordinary words of speech that Monkeytype's lists leave out (its lists are filtered): they
/// qualify without a list.
pub const SPOKEN_WORDS: &[&str] = &[
    "ok",
    "shit",
    "fuck",
    "fucking",
    "fucked",
    "ass",
    "asshole",
    "bitch",
    "bastard",
    "bullshit",
    "sex",
    "goddamn",
    "dick",
    "butt",
    "whore",
    "motherfucker",
    "pissed",
    "piss",
    "till",
    "tv",
    "dna",
    "mama",
    "papa",
    "doc",
    "lab",
    "bro",
    "nah",
    "yep",
    "yup",
    "thief",
    "darling",
    "christmas",
    "sergeant",
    "honour",
    "tip",
    "highness",
    "english",
    "spanish",
    "mum",
    "mums",
    "moms",
];
/// Monkeytype's lists that vouch for a word on their own, and the ones that also make it real
/// enough to be a word where a fragment shares its spelling.
const CORE_LISTS: [&str; 4] = ["english", "english_1k", "english_5k", "english_10k"];
const WIDE_LIST: &str = "english_25k";
const DICTIONARY: &str = "english_450k";

/// The contractions that may be typed: an inner apostrophe in any other word disqualifies it.
pub const CONTRACTIONS: &[&str] = &[
    "i'm",
    "i've",
    "i'll",
    "i'd",
    "you're",
    "you've",
    "you'll",
    "you'd",
    "he's",
    "he'll",
    "he'd",
    "she's",
    "she'll",
    "she'd",
    "it's",
    "it'll",
    "we're",
    "we've",
    "we'll",
    "we'd",
    "they're",
    "they've",
    "they'll",
    "they'd",
    "that's",
    "that'll",
    "there's",
    "here's",
    "what's",
    "who's",
    "where's",
    "how's",
    "let's",
    "don't",
    "doesn't",
    "didn't",
    "can't",
    "couldn't",
    "won't",
    "wouldn't",
    "shouldn't",
    "isn't",
    "aren't",
    "wasn't",
    "weren't",
    "hasn't",
    "haven't",
    "hadn't",
    "mustn't",
    "needn't",
    "ain't",
    "o'clock",
    "ma'am",
    "y'all",
];

/// Pieces of words and non-words: English suffixes and prefixes, letter pairs, syllables.
pub const EXTRA_FRAGMENTS: &[&str] = &[
    "er", "ed", "ly", "le", "en", "ent", "st", "ta", "kon", "eer", "ing", "ion", "ism", "ist", "ive", "ous", "ish", "ful", "est", "ity",
    "ize", "ify", "ious", "ary", "logy", "ness", "ment", "tion", "sion", "tive", "ance", "ence", "ible", "able", "ation", "al", "ic", "cy",
    "th", "re", "un", "pre", "dis", "mis", "de", "ex", "ings", "ers", "eds", "ie", "ee", "oo", "uh", "um", "ah", "hm", "sh", "ch", "ck",
    "mm", "em", "ma", "ya", "da", "pa", "jo", "la", "aw", "ow", "ha", "erm", "mr", "mrs", "dr", "lt", "lf", "ls", "ll", "ve", "gee", "ter",
    "und", "anti", "inter", "trans", "non", "semi", "multi", "ultra", "hyper", "micro", "macro", "pseudo", "poly", "mono", "neo",
];

/// Fragment texts that are also whole words of ordinary use: never a fragment.
pub const REAL_WORDS: &[&str] = &[
    "age", "less", "some", "let", "over", "under", "cross", "extra", "post", "ship", "hood", "ward", "able", "super", "mini", "auto",
    "counter", "or", "ever", "out", "up", "self", "off", "full", "free", "fore", "mid", "half", "back", "down", "high", "low", "end",
    "way", "wise", "like",
];

/// Words of an exclamation's letters: a run of three of one letter, or the shape of an
/// exclamation (`ugh`, `aah`, `hmm`, `brr`, `shh`, `psst`, `ooh`).
const EXCLAMATIONS: &[&str] = &[
    "ah", "oh", "uh", "eh", "ew", "ugh", "hm", "sh", "br", "gr", "psst", "pst", "tsk", "pft", "argh",
];

/// Whether the letters are a vocal noise: a run of three of one letter, or an exclamation's shape.
#[must_use]
pub fn noise_letters(word: &str) -> bool {
    if word.as_bytes().windows(3).any(|w| w[0] == w[1] && w[1] == w[2]) {
        return true;
    }
    let mut skeleton = String::new();
    for c in word.chars() {
        if !skeleton.ends_with(c) {
            skeleton.push(c);
        }
    }
    word.len() > 2 && EXCLAMATIONS.contains(&skeleton.as_str())
}

/// Whether the spelling is a practice word's shape: plain letters, a contraction from the list when
/// there is an apostrophe, two letters or more except `a` and `I`.
#[must_use]
pub fn shape_ok(word: &str, names: bool) -> bool {
    let lower = word.to_lowercase();
    // `won` is a contraction piece for the feed (`won't`) and also the past of `win`
    if !is_plain_word(word, names) && lower != "won" {
        return false;
    }
    if lower.contains('\'') {
        return CONTRACTIONS.contains(&lower.as_str());
    }
    lower.len() >= 2 || lower == "a" || lower == "i"
}

/// The data behind the predicate. Build it once with [`WordFilter::load`] and ask it about words.
#[derive(Clone, Debug, Default)]
pub struct WordFilter {
    /// Monkeytype's english, 1k, 5k and 10k words
    core: HashSet<String>,
    /// Monkeytype's 25k words
    wide: HashSet<String>,
    /// Monkeytype's 450k dictionary (empty when not downloaded)
    full: HashSet<String>,
    /// Plain single words of the user's dictionaries and word lists
    user: HashSet<String>,
    fragments: HashSet<String>,
    /// `--deep`: any word of the 450k dictionary
    pub deep: bool,
    /// `--names`: capitalised names are allowed
    pub allow_names: bool,
}

impl WordFilter {
    /// Reads what the predicate needs from the data folder; missing files only mean less is known.
    /// `user` are the words of the user's dictionaries and lists (lowercase).
    #[must_use]
    pub fn load(data: &Path, deep: bool, allow_names: bool, user: HashSet<String>) -> Self {
        let read = |name: &str| -> HashSet<String> {
            std::fs::read_to_string(list_path(data, name))
                .ok()
                .and_then(|t| parse_monkeytype(&t).ok())
                // only the entries written lowercase: the lists keep names capitalised (`Sam`)
                .map_or_else(HashSet::new, |w| {
                    w.into_iter()
                        .filter(|w| !w.chars().any(char::is_uppercase) || w == "I")
                        .map(|w| w.to_lowercase())
                        .collect()
                })
        };
        let mut core = HashSet::new();
        for name in CORE_LISTS {
            core.extend(read(name));
        }
        if core.is_empty() {
            core.extend(super::pool::embedded_english().into_iter().map(|w| w.to_lowercase()));
        }
        Self {
            core,
            wide: read(WIDE_LIST),
            full: read(DICTIONARY),
            user,
            fragments: EXTRA_FRAGMENTS.iter().map(|s| (*s).to_owned()).collect(),
            deep,
            allow_names,
        }
    }

    #[cfg(test)]
    /// A filter from given sets, for tests: `core` words, `wide` (25k) words, `full` (450k) words
    /// and the user's words.
    #[must_use]
    pub fn from_parts(core: &[&str], wide: &[&str], full: &[&str], user: &[&str], deep: bool) -> Self {
        let set = |w: &[&str]| w.iter().map(|w| (*w).to_owned()).collect();
        Self {
            core: set(core),
            wide: set(wide),
            full: set(full),
            user: set(user),
            fragments: EXTRA_FRAGMENTS.iter().map(|s| (*s).to_owned()).collect(),
            deep,
            allow_names: false,
        }
    }

    /// Whether the word has the shape of a practice word but is a fragment.
    #[must_use]
    pub fn is_fragment(&self, word: &str) -> bool {
        let w = word.to_lowercase();
        self.fragments.contains(&w) && !self.core.contains(&w) && !REAL_WORDS.contains(&w.as_str())
    }

    /// Whether `word` is a practice word (see the module note).
    #[must_use]
    pub fn accepts(&self, word: &str) -> bool {
        if !shape_ok(word, self.allow_names) {
            return false;
        }
        let w = word.to_lowercase();
        if self.is_fragment(&w) || noise_letters(&w) {
            return false;
        }
        if matches!(w.as_str(), "a" | "i") || w.contains('\'') || self.core.contains(&w) || self.wide.contains(&w) || self.user.contains(&w)
        {
            return true;
        }
        if SPOKEN_WORDS.contains(&w.as_str()) {
            return true;
        }
        self.deep && (self.full.is_empty() || self.full.contains(&w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(deep: bool) -> WordFilter {
        WordFilter::from_parts(
            &["the", "age", "less", "over", "or", "don't", "it's", "i'm", "a", "see"],
            &["quokka"],
            &[
                "tash", "eer", "er", "ed", "ing", "ly", "le", "en", "st", "kon", "ent", "abb", "ack", "ordinary", "rare", "ish", "hmm",
                "zebra", "ash", "ode",
            ],
            &["gizmo"],
            deep,
        )
    }

    #[test]
    fn real_words_pass_and_the_junk_of_a_real_vocab_fails() {
        let f = filter(false);
        assert!(f.accepts("it's") && f.accepts("don't"), "contractions are not in any list");
        for ok in [
            "the", "a", "I", "i", "ok", "shit", "age", "less", "over", "or", "don't", "it's", "I'm", "quokka", "see", "gizmo",
        ] {
            assert!(f.accepts(ok), "{ok} should qualify");
        }
        for bad in [
            "tash", "eer", "er", "ed", "ing", "ly", "le", "en", "st", "ta", "kon", "ent", "ab", "abb", "ack", "ahn", "ben", "hmm", "ish",
            "zzz", "ugh", "", "x", "co2", "NASA", "Matt", "can'tt", "o'", "'s", "john's", "ode", "ash", "ordinary",
        ] {
            assert!(!f.accepts(bad), "{bad} should not qualify");
        }
    }

    #[test]
    fn a_dictionary_word_alone_is_not_enough_without_deep() {
        let f = filter(false);
        assert!(!f.accepts("ordinary") && !f.accepts("rare"), "in the 450k dictionary only");
        assert!(filter(true).accepts("ordinary"));
    }

    #[test]
    fn deep_adds_the_dictionary_and_keeps_fragments_and_noise_out() {
        let f = filter(true);
        assert!(f.accepts("rare") && f.accepts("zebra") && f.accepts("ash"), "any dictionary word");
        assert!(f.accepts("tash"), "deep reaches the dictionary tail");
        for bad in ["er", "ed", "ing", "ly", "le", "en", "st", "kon", "ent", "eer", "ish", "hmm"] {
            assert!(!f.accepts(bad), "{bad} is still out with --deep");
        }
        assert!(!f.accepts("ahn"), "not in the dictionary");
        assert!(
            WordFilter::from_parts(&[], &[], &[], &[], true).accepts("anything"),
            "no dictionary downloaded: deep lets any plain word in"
        );
    }

    #[test]
    fn contractions_are_a_fixed_list() {
        for c in CONTRACTIONS {
            assert!(shape_ok(c, false), "{c}");
        }
        for bad in ["abc'd", "dogs'", "tash's", "it'ss", "'tis"] {
            assert!(!shape_ok(bad, false), "{bad}");
        }
        assert!(shape_ok("I'm", false) && shape_ok("I", false) && shape_ok("a", false) && !shape_ok("b", false));
        assert!(
            shape_ok("won", false) && !shape_ok("don", false),
            "won is a word; don is a piece of don't"
        );
    }

    #[test]
    fn exclamation_shapes_are_noise_and_words_are_not() {
        for word in ["ugh", "aah", "hmm", "brr", "zzz", "shh", "mmm", "psst", "ooh", "ahh", "eww", "pfft"] {
            assert!(noise_letters(word), "{word} should be noise");
        }
        for word in [
            "ease", "yeah", "oh", "ah", "ok", "thing", "mr", "tv", "good", "off", "see", "ash", "shy",
        ] {
            assert!(!noise_letters(word), "{word} should not be noise");
        }
    }

    #[test]
    fn every_fragment_is_a_fragment_unless_it_is_a_real_word() {
        let f = filter(false);
        for t in [
            "ing", "ed", "ly", "er", "en", "ent", "ness", "ment", "tion", "ism", "ist", "anti", "inter", "trans", "dis", "mis", "non",
            "ish", "ize",
        ] {
            assert!(f.is_fragment(t), "{t}");
        }
        for t in ["age", "less", "over", "under", "or", "some", "let"] {
            assert!(!f.is_fragment(t), "{t} is a real word too");
        }
    }
}
