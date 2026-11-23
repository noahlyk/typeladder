//! The word pool of the adaptive source: Monkeytype's English lists and the user's own word lists,
//! merged, deduplicated and put in one order that the adaptive engine walks down.
//!
//! The order, from the commonest word down:
//!
//! 1. Monkeytype's frequency-ordered lists `english` (200), `english_1k`, `english_5k` and
//!    `english_10k`, each adding the words an earlier list does not have, in the list's own order;
//! 2. `english_25k`, whose file says `orderedByFrequency: false`: a fixed scrambled order (a hash
//!    of the word), not alphabetical;
//! 3. the user's word lists (`lists` in `config.toml`), each in its file's order;
//! 4. with `--deep`, `english_450k`, the unordered dictionary, scrambled the same way.
//!
//! The head (1) is built when the pool opens, a few milliseconds. A tail list is read only when
//! the engine walks past what is loaded. The 200-word list is built into the program, so a pool
//! always has a head, even before anything is downloaded.

use super::words::WordFilter;
use anyhow::{Result, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The frequency-ordered Monkeytype lists, in tier order.
pub const ORDERED: [&str; 4] = ["english", "english_1k", "english_5k", "english_10k"];
/// The unordered lists read as tiers after the head: the 25k word list, and (with `--deep`) the
/// 450k dictionary.
const WIDE: &str = "english_25k";
const DEEP: &str = "english_450k";

/// The 200 commonest words, built in (Monkeytype's `english` list).
const EMBEDDED_ENGLISH: &str = include_str!("../../assets/monkeytype-english.json");

/// A word list of the user's: a tier of its own after Monkeytype's 25k.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserList {
    /// The key a goal is stored under (`list:mine`)
    pub key: String,
    pub label: String,
    pub words: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Opts {
    pub names: bool,
    /// `--deep`: the whole 450k dictionary
    pub deep: bool,
    /// The user's word lists
    pub lists: Vec<UserList>,
    /// Words of the user's dictionaries: they count as practice words
    pub user_words: HashSet<String>,
}

/// Where a Monkeytype list is saved by `typeladder fetch`.
#[must_use]
pub fn list_path(data: &Path, name: &str) -> PathBuf {
    data.join("wordlists").join(format!("monkeytype-{name}.json"))
}

/// The words of a Monkeytype language file: `{"name": .., "words": [..]}` (or a bare array), in
/// file order.
pub fn parse_monkeytype(json: &str) -> Result<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let words = match &value {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(o) => match o.get("words") {
            Some(serde_json::Value::Array(a)) => a,
            _ => bail!("no \"words\" array"),
        },
        _ => bail!("not a word list"),
    };
    Ok(words.iter().filter_map(|w| w.as_str().map(str::to_owned)).collect())
}

/// The built-in 200 commonest words.
#[must_use]
pub fn embedded_english() -> Vec<String> {
    parse_monkeytype(EMBEDDED_ENGLISH).unwrap_or_default()
}

/// The words of a Monkeytype list: the file in the data folder, or the built-in copy for `english`.
#[must_use]
pub fn read_list(data: &Path, name: &str) -> Option<Vec<String>> {
    match std::fs::read_to_string(list_path(data, name))
        .ok()
        .and_then(|t| parse_monkeytype(&t).ok())
    {
        Some(words) => Some(words),
        None if name == "english" => Some(embedded_english()),
        None => None,
    }
}

/// A list's usable words (practice words by `filter`), in order, each once.
pub(super) fn usable(words: Vec<String>, filter: &WordFilter) -> Vec<String> {
    let mut seen = HashSet::new();
    words
        .into_iter()
        .map(|w| if w == "I" { "i".to_owned() } else { w })
        .filter(|w| filter.accepts(w) && seen.insert(w.clone()))
        .collect()
}

/// The head of the pool in order: each list's words that no earlier list has.
#[must_use]
pub fn head(lists: &[(String, Vec<String>)]) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for (_, words) in lists {
        for w in words {
            if seen.insert(w.as_str()) {
                out.push(w.clone());
            }
        }
    }
    out
}

/// A fixed scrambling of a word for the unordered tail (FNV-1a).
fn scramble(word: &str) -> u64 {
    word.bytes()
        .fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The unordered tail: `words` not already `taken`, scrambled.
#[must_use]
pub fn tail(words: Vec<String>, taken: &HashSet<String>) -> Vec<String> {
    let mut v: Vec<String> = words.into_iter().filter(|w| !taken.contains(w)).collect();
    v.sort_by(|a, b| scramble(a).cmp(&scramble(b)).then(a.cmp(b)));
    v.dedup();
    v
}

/// Words in pool order, fetched by index; the pool loads more behind the call as it is walked.
pub trait Candidates {
    fn get(&mut self, i: usize) -> Option<&str>;
}

impl Candidates for Vec<String> {
    fn get(&mut self, i: usize) -> Option<&str> {
        self.as_slice().get(i).map(String::as_str)
    }
}

/// What a tail stage reads.
#[derive(Clone, Debug)]
enum Part {
    /// An unordered Monkeytype list, scrambled
    Scrambled(&'static str),
    /// A user list, in its own order (index into the lists)
    User(usize),
}

pub struct Pool {
    words: Vec<String>,
    /// The tail parts not read yet, in order
    parts: Vec<Part>,
    next: usize,
    data: PathBuf,
    lists: Vec<UserList>,
    /// The practice-word predicate every word of the pool passed
    pub filter: std::sync::Arc<WordFilter>,
    /// Lists wanted and not downloaded (`typeladder fetch`)
    pub missing: Vec<String>,
}

impl Pool {
    /// Opens the pool: reads the head only.
    #[must_use]
    pub fn open(data: &Path, opts: &Opts) -> Self {
        let mut user_words = opts.user_words.clone();
        user_words.extend(opts.lists.iter().flat_map(|l| l.words.iter().map(|w| w.to_lowercase())));
        let filter = std::sync::Arc::new(WordFilter::load(data, opts.deep, opts.names, user_words));
        let mut missing = Vec::new();
        let mut lists = Vec::new();
        for name in ORDERED {
            match read_list(data, name) {
                Some(words) => lists.push((name.to_owned(), usable(words, &filter))),
                None => missing.push(name.to_owned()),
            }
        }
        let mut parts = vec![Part::Scrambled(WIDE)];
        parts.extend((0..opts.lists.len()).map(Part::User));
        if opts.deep {
            parts.push(Part::Scrambled(DEEP));
        }
        Self {
            words: head(&lists),
            parts,
            next: 0,
            data: data.to_path_buf(),
            lists: opts.lists.clone(),
            filter,
            missing,
        }
    }

    /// Loads the next part of the pool; false when there is none.
    fn more(&mut self) -> bool {
        while self.next < self.parts.len() {
            let part = self.parts[self.next].clone();
            self.next += 1;
            let tier = match part {
                Part::Scrambled(name) => {
                    let Some(words) = read_list(&self.data, name) else {
                        self.missing.push(name.to_owned());
                        continue;
                    };
                    let taken: HashSet<String> = self.words.iter().cloned().collect();
                    tail(usable(words, &self.filter), &taken)
                }
                Part::User(i) => {
                    let taken: HashSet<&String> = self.words.iter().collect();
                    let mut seen = HashSet::new();
                    usable(self.lists[i].words.clone(), &self.filter)
                        .into_iter()
                        .filter(|w| !taken.contains(w) && seen.insert(w.clone()))
                        .collect()
                }
            };
            if !tier.is_empty() {
                self.words.extend(tier);
                return true;
            }
        }
        false
    }

    /// The tiers of this pool's data (see `tiers`); `deep` adds the 450k dictionary's tail.
    #[must_use]
    pub fn tiers(&self, deep: bool) -> Vec<super::tiers::Tier> {
        super::tiers::load(&self.data, &self.filter, &self.lists, deep)
    }

    /// Words loaded so far.
    #[cfg(test)]
    #[must_use]
    pub const fn loaded(&self) -> usize {
        self.words.len()
    }
}

impl Candidates for Pool {
    fn get(&mut self, i: usize) -> Option<&str> {
        while i >= self.words.len() {
            if !self.more() {
                return None;
            }
        }
        self.words.as_slice().get(i).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn both_monkeytype_formats_parse() {
        let object = r#"{"name":"english","noLazyMode":true,"orderedByFrequency":true,"words":["the","be","of"]}"#;
        assert_eq!(parse_monkeytype(object).unwrap(), ["the", "be", "of"]);
        assert_eq!(parse_monkeytype(r#"["a","b"]"#).unwrap(), ["a", "b"]);
        assert_eq!(
            parse_monkeytype(r#"{"words":["x",3,"y"]}"#).unwrap(),
            ["x", "y"],
            "non-strings are dropped"
        );
        assert!(parse_monkeytype(r#"{"name":"x"}"#).is_err());
        assert!(parse_monkeytype("not json").is_err());
        assert!(parse_monkeytype("3").is_err());
    }

    #[test]
    fn the_built_in_list_is_the_two_hundred_commonest_words() {
        let words = embedded_english();
        assert_eq!(words.len(), 200);
        assert_eq!(words[0], "the");
    }

    #[test]
    fn usable_words_are_practice_words_and_unique() {
        let f = WordFilter::from_parts(&["the", "don't", "matt", "ben", "co"], &[], &[], &[], false);
        assert_eq!(
            usable(v(&["the", "Matt", "NASA", "the", "co2", "don't", "ben"]), &f),
            ["the", "don't", "ben"]
        );
        let mut with_names = f;
        with_names.allow_names = true;
        assert_eq!(usable(v(&["Matt"]), &with_names), ["Matt"]);
    }

    #[test]
    fn the_head_is_each_list_in_turn_without_repeats() {
        let lists = [
            ("english".to_owned(), v(&["the", "of"])),
            ("english_1k".to_owned(), v(&["of", "and", "the", "cat"])),
        ];
        assert_eq!(head(&lists), ["the", "of", "and", "cat"]);
    }

    #[test]
    fn the_unordered_tail_leaves_out_the_head_and_is_not_alphabetical() {
        let words: Vec<String> = (0..200).map(|i| format!("w{i:03}")).collect();
        let taken: HashSet<String> = words.iter().take(10).cloned().collect();
        let t = tail(words.clone(), &taken);
        assert_eq!(t.len(), 190);
        assert!(t.iter().all(|w| !taken.contains(w)));
        let mut sorted = t.clone();
        sorted.sort();
        assert_ne!(t, sorted, "scrambled, not alphabetical");
        assert_eq!(t, tail(words, &taken), "and the same every time");
    }

    fn data_dir(name: &str, with_lists: bool) -> PathBuf {
        let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.join(format!("typeladder-pool-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("wordlists")).unwrap();
        if with_lists {
            std::fs::write(
                list_path(&dir, "english"),
                r#"{"name":"english","words":["the","dog","apple","of","and"]}"#,
            )
            .unwrap();
            std::fs::write(
                list_path(&dir, "english_1k"),
                r#"{"name":"english_1k","words":["the","cat","dog","house"]}"#,
            )
            .unwrap();
            std::fs::write(
                list_path(&dir, "english_450k"),
                r#"{"name":"english_450k","words":["Aaron","apple","zebra","yak","the","cat"]}"#,
            )
            .unwrap();
            std::fs::write(
                list_path(&dir, "english_25k"),
                r#"{"name":"english_25k","orderedByFrequency":false,"words":["quokka","yak","cat"]}"#,
            )
            .unwrap();
        }
        dir
    }

    fn walk(pool: &mut Pool) -> Vec<String> {
        let mut all = Vec::new();
        let mut i = 0;
        while let Some(w) = pool.get(i) {
            all.push(w.to_owned());
            i += 1;
        }
        all
    }

    #[test]
    fn the_pool_walks_head_then_the_lazy_tail_and_never_repeats() {
        let dir = data_dir("walk", true);
        let mut pool = Pool::open(&dir, &Opts::default());
        // english: the dog apple of and; 1k adds cat house; the 450k dictionary is not read yet
        assert_eq!(pool.loaded(), 7);
        let all = walk(&mut pool);
        assert_eq!(&all[..7], ["the", "dog", "apple", "of", "and", "cat", "house"]);
        assert_eq!(all.len(), 9, "the 25k list adds quokka and yak: {all:?}");
        let unique: HashSet<&String> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "{all:?}");
        assert!(!all.contains(&"zebra".to_owned()), "a dictionary-only word is for --deep: {all:?}");
        let mut deep = Pool::open(
            &dir,
            &Opts {
                deep: true,
                ..Opts::default()
            },
        );
        let deep_all = walk(&mut deep);
        let at = |w: &str| deep_all.iter().position(|x| x == w).unwrap();
        assert!(at("quokka") < at("zebra"), "the 25k tier comes before the 450k one: {deep_all:?}");
        assert!(!deep_all.contains(&"Aaron".to_owned()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_lists_come_after_the_25k_and_dictionary_words_are_practice_words() {
        let dir = data_dir("user", true);
        let list = UserList {
            key: "list:mine".into(),
            label: "list mine".into(),
            words: v(&["gizmo", "the", "apple", "x1"]),
        };
        let mut pool = Pool::open(
            &dir,
            &Opts {
                lists: vec![list],
                ..Opts::default()
            },
        );
        let all = walk(&mut pool);
        assert_eq!(all[all.len() - 1], "gizmo", "{all:?}");
        assert!(all.iter().position(|w| w == "quokka").unwrap() < all.iter().position(|w| w == "gizmo").unwrap());
        assert_eq!(
            all.iter().filter(|w| *w == "the" || *w == "apple").count(),
            2,
            "words of earlier tiers are not repeated"
        );
        assert!(!all.contains(&"x1".to_owned()), "not a plain word");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_nothing_downloaded_the_built_in_list_is_the_pool() {
        let bare = data_dir("bare", false);
        let mut pool = Pool::open(&bare, &Opts::default());
        assert_eq!(pool.get(0), Some("the"));
        let n = walk(&mut pool).len();
        assert!(n >= 190, "the built-in 200 less what the filter drops: {n}");
        assert_eq!(pool.missing, ["english_1k", "english_5k", "english_10k", "english_25k"]);
        let _ = std::fs::remove_dir_all(&bare);
    }
}
