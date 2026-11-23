//! `config.toml`: everything is optional, and no file at all is a working setup (a plain typing
//! test over Monkeytype's word lists).
//!
//! ```toml
//! accept = ["text"]                 # or ["text", "code"]
//! hints = false                     # codes beside the stumbled words on the results screen
//! phrases = false                   # the dictionaries' multiword entries as practice items
//! auto_fetch = true                 # download Monkeytype's lists on the first run
//! lists = ["~/words.txt"]           # your own word lists, a tier each, after Monkeytype's 25k
//!
//! [seed]
//! usage = "~/usage.txt"             # "word attempts errors [days]" lines, read once on first run
//!
//! [hooks]                           # sh -c, 3 seconds each, a failure is a warning
//! before_session = "espanso stop"
//! after_session = "espanso start"
//! before_test = ""
//! after_test = ""
//!
//! [dictionary."lyk"]
//! direction = "codes"               # or "texts": overrides the file name and the guess
//! ```

use crate::dict::Direction;
use crate::practice::pool::{UserList, parse_monkeytype};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// What counts as a right answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Accept {
    /// The word itself
    Text,
    /// One of its codes from the dictionaries
    Code,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Hooks {
    pub before_session: Option<String>,
    pub after_session: Option<String>,
    pub before_test: Option<String>,
    pub after_test: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Seed {
    /// A usage file read once when the vocabulary is first made
    pub usage: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DictionaryCfg {
    pub direction: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub accept: Vec<Accept>,
    pub hints: bool,
    pub phrases: bool,
    pub auto_fetch: bool,
    pub lists: Vec<String>,
    pub seed: Seed,
    pub hooks: Hooks,
    pub dictionary: BTreeMap<String, DictionaryCfg>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            accept: vec![Accept::Text],
            hints: false,
            phrases: false,
            auto_fetch: true,
            lists: Vec::new(),
            seed: Seed::default(),
            hooks: Hooks::default(),
            dictionary: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Parses config text (the file's contents).
    pub fn parse(text: &str) -> Result<Self> {
        let cfg: Self = toml::from_str(text)?;
        if !cfg.accept.contains(&Accept::Text) {
            bail!("accept must include \"text\" (the word itself is always a right answer)");
        }
        for (name, d) in &cfg.dictionary {
            if let Some(dir) = &d.direction
                && Direction::parse(dir).is_none()
            {
                bail!("[dictionary.\"{name}\"] direction = {dir:?}: write \"codes\" or \"texts\"");
            }
        }
        Ok(cfg)
    }

    /// Reads the file; a missing file is the defaults.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| path.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| path.display().to_string()),
        }
    }

    /// Whether a code typed for a word counts.
    #[must_use]
    pub fn accepts_code(&self) -> bool {
        self.accept.contains(&Accept::Code)
    }

    /// The per-dictionary direction overrides.
    #[must_use]
    pub fn directions(&self) -> BTreeMap<String, Direction> {
        self.dictionary
            .iter()
            .filter_map(|(n, d)| d.direction.as_deref().and_then(Direction::parse).map(|d| (n.clone(), d)))
            .collect()
    }
}

/// The words of a list file: Monkeytype's JSON (`.json`), otherwise one or more words a line with
/// `#` comments.
fn read_words(path: &Path, text: &str) -> Result<Vec<String>> {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
        return parse_monkeytype(text);
    }
    Ok(text
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default())
        .flat_map(str::split_whitespace)
        .map(str::to_owned)
        .collect())
}

/// The user's word lists from `lists`; one that cannot be read is a warning.
#[must_use]
pub fn load_lists(paths: &[String]) -> (Vec<UserList>, Vec<String>) {
    let (mut lists, mut warnings) = (Vec::new(), Vec::new());
    for p in paths {
        let path = crate::paths::expand_home(Path::new(p));
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("list").to_owned();
        match std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|t| read_words(&path, &t))
        {
            Ok(words) => lists.push(UserList {
                key: format!("list:{stem}"),
                label: format!("list {stem}"),
                words,
            }),
            Err(e) => warnings.push(format!("word list skipped: {}: {e:#}", path.display())),
        }
    }
    (lists, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_file_and_an_empty_file_are_the_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert_eq!(Config::load(Path::new("/no/such/config.toml")).unwrap(), Config::default());
        let d = Config::default();
        assert!(!d.hints && !d.phrases && d.auto_fetch && !d.accepts_code());
    }

    #[test]
    fn every_key_reads() {
        let cfg = Config::parse(
            "accept = [\"text\", \"code\"]\nhints = true\nphrases = true\nauto_fetch = false\nlists = [\"~/w.txt\"]\n[seed]\nusage = \"u.txt\"\n[hooks]\nbefore_session = \"espanso stop\"\nafter_session = \"espanso start\"\nbefore_test = \"a\"\nafter_test = \"b\"\n[dictionary.\"lyk\"]\ndirection = \"codes\"\n",
        )
        .unwrap();
        assert!(cfg.accepts_code() && cfg.hints && cfg.phrases && !cfg.auto_fetch);
        assert_eq!(cfg.lists, ["~/w.txt"]);
        assert_eq!(cfg.seed.usage.as_deref(), Some("u.txt"));
        assert_eq!(cfg.hooks.before_session.as_deref(), Some("espanso stop"));
        assert_eq!(cfg.hooks.after_test.as_deref(), Some("b"));
        assert_eq!(cfg.directions().get("lyk"), Some(&Direction::Codes));
    }

    #[test]
    fn mistakes_are_errors_that_name_the_problem() {
        assert!(Config::parse("hint = true").unwrap_err().to_string().contains("hint"));
        assert!(Config::parse("accept = [\"code\"]").unwrap_err().to_string().contains("text"));
        assert!(
            Config::parse("[dictionary.\"x\"]\ndirection = \"up\"")
                .unwrap_err()
                .to_string()
                .contains("codes")
        );
        assert!(Config::parse("accept = [\"word\"]").is_err());
    }

    #[test]
    fn word_lists_read_as_text_or_monkeytype_json() {
        let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
        let dir = base.join(format!("typeladder-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mine.txt"), "alpha beta # a comment\ngamma\n# only a comment\n").unwrap();
        std::fs::write(dir.join("j.json"), r#"{"words": ["delta"]}"#).unwrap();
        let paths: Vec<String> = ["mine.txt", "j.json", "gone.txt"]
            .iter()
            .map(|f| dir.join(f).to_string_lossy().into_owned())
            .collect();
        let (lists, warnings) = load_lists(&paths);
        assert_eq!(lists.len(), 2);
        assert_eq!(
            (lists[0].key.as_str(), lists[0].words.as_slice()),
            ("list:mine", &["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned()][..])
        );
        assert_eq!(lists[1].words, ["delta"]);
        assert_eq!(warnings.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
