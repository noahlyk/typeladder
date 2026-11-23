//! The user's own dictionaries: shortcut, brief or outline tables that map a word to the code the
//! typist types for it (a steno outline, an espanso trigger, a text-expander abbreviation).
//!
//! Every `.json`, `.yaml`, `.yml` or `.toml` file in the dictionaries folder is one dictionary.
//! The program never needs one: they give *hints* (shown on the results screen on request and by
//! `typeladder lookup`), the *phrases* of `--phrases`, and the extra *accepted answers* of accept
//! mode.
//!
//! ## Shapes read
//!
//! 1. a flat map `{"a": "b"}` (a value may be a list of strings: several codes for one word);
//! 2. an espanso match file (YAML with a `matches:` list of `{trigger | triggers, replace}`);
//! 3. a TOML table (nested tables are flattened);
//! 4. a list of records `{from, to}`, `{key, value}` or `{trigger | triggers, replace}`.
//!
//! ## Direction
//!
//! Each file is *text to code* (`{"create": "KRAET"}`, `codes`) or *code to text* (`{"KRAET":
//! "create"}`, `texts`, the Plover and lyk way). In order of priority:
//!
//! 1. `[dictionary."NAME"] direction = "codes" | "texts"` in `config.toml`;
//! 2. the file name: `NAME.codes.EXT` is `codes`, `NAME.texts.EXT` is `texts`;
//! 3. the format: espanso matches are always `texts` (trigger is the code);
//! 4. a guess ([`guess`]): if at least 60% of the keys look like strokes (no spaces, capital
//!    letters, digits and stroke marks only) and fewer values do, `texts`; the reverse, `codes`;
//!    otherwise the shorter side is the code (a trigger is shorter than what it expands to), and
//!    when the sides are about as long, a tie, `texts`.
//!
//! `typeladder lookup` says which rule decided each file.

// Counts become shares: the casts are exact at these sizes.
#![allow(clippy::cast_precision_loss)]

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What a dictionary's keys are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// `{"create": "KRAET"}`: the key is the text, the value its code
    Codes,
    /// `{"KRAET": "create"}`: the key is the code, the value its text
    Texts,
}

impl Direction {
    /// `codes` or `texts`, as written in `config.toml` and file names.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Codes => "codes",
            Self::Texts => "texts",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "codes" => Some(Self::Codes),
            "texts" => Some(Self::Texts),
            _ => None,
        }
    }
}

/// What decided a dictionary's direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Config,
    FileName,
    Format,
    /// Guessed; the flag says it was a tie
    Guess(bool),
}

impl Origin {
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Config => "set in config.toml",
            Self::FileName => "from the file name",
            Self::Format => "from the file format",
            Self::Guess(false) => "guessed",
            Self::Guess(true) => "guessed: a tie, preferring code to text",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Dictionary {
    pub name: String,
    pub direction: Direction,
    pub origin: Origin,
    /// `(text, code)` of each entry, in file order
    pub entries: Vec<(String, String)>,
}

/// All the user's dictionaries, with an index from text to codes.
#[derive(Clone, Debug, Default)]
pub struct Dictionaries {
    pub list: Vec<Dictionary>,
    /// What could not be read, one line each
    pub warnings: Vec<String>,
    /// lowercase text -> (dictionary index, code)
    by_text: HashMap<String, Vec<(usize, String)>>,
    /// code -> (dictionary index, text)
    by_code: HashMap<String, Vec<(usize, String)>>,
}

/// Entries read from one file, before the direction is known.
#[derive(Debug, PartialEq, Eq)]
struct Parsed {
    /// `(left, right)` as written: key and value, `from` and `to`, trigger and replace
    pairs: Vec<(String, String)>,
    /// The format fixes the direction (espanso)
    fixed: Option<Direction>,
}

const MATCH_TRIGGER: [&str; 2] = ["trigger", "triggers"];

fn strings_of(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    }
}

/// `{trigger | triggers, replace}` records, left the trigger (a code), right the replacement.
fn match_pairs(items: &[Value]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for item in items {
        let Some(obj) = item.as_object() else { continue };
        let Some(replace) = obj.get("replace").and_then(Value::as_str) else {
            continue;
        };
        for key in MATCH_TRIGGER {
            if let Some(v) = obj.get(key) {
                out.extend(strings_of(v).into_iter().map(|t| (t, replace.to_owned())));
            }
        }
    }
    out
}

/// Pairs out of a map: string values and lists of strings; nested maps are flattened.
fn flat_pairs(map: &serde_json::Map<String, Value>, out: &mut Vec<(String, String)>, depth: usize) {
    for (k, v) in map {
        match v {
            Value::String(_) | Value::Array(_) => out.extend(strings_of(v).into_iter().map(|s| (k.clone(), s))),
            Value::Object(inner) if depth < 3 => flat_pairs(inner, out, depth + 1),
            _ => {}
        }
    }
}

fn record_pairs(items: &[Value]) -> Parsed {
    let mut pairs = Vec::new();
    let mut espanso = false;
    for item in items {
        let Some(obj) = item.as_object() else { continue };
        if obj.contains_key("replace") {
            espanso = true;
            pairs.extend(match_pairs(std::slice::from_ref(item)));
            continue;
        }
        for (l, r) in [("from", "to"), ("key", "value")] {
            if let (Some(a), Some(b)) = (obj.get(l), obj.get(r)) {
                for (x, y) in strings_of(a)
                    .into_iter()
                    .flat_map(|x| strings_of(b).into_iter().map(move |y| (x.clone(), y)))
                {
                    pairs.push((x, y));
                }
            }
        }
    }
    Parsed {
        pairs,
        fixed: espanso.then_some(Direction::Texts),
    }
}

fn parse_value(value: &Value) -> Result<Parsed> {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("matches") {
                return Ok(Parsed {
                    pairs: match_pairs(items),
                    fixed: Some(Direction::Texts),
                });
            }
            let mut pairs = Vec::new();
            flat_pairs(map, &mut pairs, 0);
            Ok(Parsed { pairs, fixed: None })
        }
        Value::Array(items) => Ok(record_pairs(items)),
        _ => bail!("not a table of entries"),
    }
}

/// Reads a file's text by its extension (`json`, `yaml`, `yml`, `toml`) into the generic shape.
fn read_value(path: &Path, text: &str) -> Result<Value> {
    match path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
        Some("json") => Ok(serde_json::from_str(text)?),
        Some("yaml" | "yml") => Ok(serde_norway::from_str(text)?),
        Some("toml") => Ok(toml::from_str(text)?),
        _ => bail!("not a .json, .yaml, .yml or .toml file"),
    }
}

/// Whether `s` looks like a steno stroke or other code: no spaces, at least one capital letter,
/// no lowercase letter, and nothing but capitals, digits and the marks strokes use.
#[must_use]
pub fn stroke_like(s: &str) -> bool {
    !s.is_empty()
        && s.chars().any(|c| c.is_ascii_uppercase())
        && s.chars().all(|c| {
            c.is_ascii_uppercase()
                || c.is_ascii_digit()
                || matches!(c, '-' | '*' | '/' | '#' | '^' | '+' | '~' | '$' | '&' | '@' | '%' | ':' | '\\')
        })
}

/// The direction of a file with no name, config or format to say: see the module note. The flag is
/// true for a tie.
#[must_use]
pub fn guess(pairs: &[(String, String)]) -> (Direction, bool) {
    if pairs.is_empty() {
        return (Direction::Texts, true);
    }
    let n = pairs.len() as f64;
    let share = |f: &dyn Fn(&(String, String)) -> bool| pairs.iter().filter(|p| f(p)).count() as f64 / n;
    let (k, v) = (share(&|p| stroke_like(&p.0)), share(&|p| stroke_like(&p.1)));
    if k >= 0.6 && v < 0.6 {
        return (Direction::Texts, false);
    }
    if v >= 0.6 && k < 0.6 {
        return (Direction::Codes, false);
    }
    let avg = |f: &dyn Fn(&(String, String)) -> &str| pairs.iter().map(|p| f(p).chars().count()).sum::<usize>() as f64 / n;
    let (kl, vl) = (avg(&|p| &p.0), avg(&|p| &p.1));
    if kl < vl * 0.8 {
        (Direction::Texts, false)
    } else if kl > vl * 1.25 {
        (Direction::Codes, false)
    } else {
        (Direction::Texts, true)
    }
}

/// `NAME.codes.json` is `("NAME", Some(Codes))`; `NAME.json` is `("NAME", None)`.
fn name_of(file: &str) -> (String, Option<Direction>) {
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    for (suffix, d) in [(".codes", Direction::Codes), (".texts", Direction::Texts)] {
        if let Some(name) = stem.strip_suffix(suffix).filter(|n| !n.is_empty()) {
            return (name.to_owned(), Some(d));
        }
    }
    (stem.to_owned(), None)
}

/// One file's dictionary.
fn read_one(path: &Path, overrides: &BTreeMap<String, Direction>) -> Result<Dictionary> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let value = read_value(path, &text).with_context(|| path.display().to_string())?;
    let parsed = parse_value(&value).with_context(|| path.display().to_string())?;
    let file = path.file_name().and_then(|f| f.to_str()).unwrap_or_default();
    let (name, named) = name_of(file);
    let (direction, origin) = if let Some(d) = overrides.get(&name) {
        (*d, Origin::Config)
    } else if let Some(d) = named {
        (d, Origin::FileName)
    } else if let Some(d) = parsed.fixed {
        (d, Origin::Format)
    } else {
        let (d, tie) = guess(&parsed.pairs);
        (d, Origin::Guess(tie))
    };
    let entries = parsed
        .pairs
        .into_iter()
        .map(|(l, r)| {
            let (text, code) = if direction == Direction::Codes { (l, r) } else { (r, l) };
            (text.trim().to_owned(), code.trim().to_owned())
        })
        .filter(|(t, c)| !t.is_empty() && !c.is_empty())
        .collect();
    Ok(Dictionary {
        name,
        direction,
        origin,
        entries,
    })
}

impl Dictionaries {
    /// Reads every dictionary file in `dir` (a missing folder is no dictionaries). A file that
    /// cannot be read is a warning, never an error.
    #[must_use]
    pub fn load(dir: &Path, overrides: &BTreeMap<String, Direction>) -> Self {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| matches!(e.to_lowercase().as_str(), "json" | "yaml" | "yml" | "toml"))
            })
            .collect();
        files.sort();
        let mut out = Self::default();
        for path in files {
            match read_one(&path, overrides) {
                Ok(d) => out.list.push(d),
                Err(e) => out.warnings.push(format!("dictionary skipped: {e:#}")),
            }
        }
        out.index();
        out
    }

    fn index(&mut self) {
        for (i, d) in self.list.iter().enumerate() {
            for (text, code) in &d.entries {
                self.by_text.entry(text.to_lowercase()).or_default().push((i, code.clone()));
                self.by_code.entry(code.clone()).or_default().push((i, text.clone()));
            }
        }
    }

    /// Each dictionary's codes for `text` (case-insensitive), fewest strokes then shortest first.
    #[must_use]
    pub fn codes_for(&self, text: &str) -> Vec<(&Dictionary, Vec<String>)> {
        let mut per: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (i, code) in self.by_text.get(&text.to_lowercase()).into_iter().flatten() {
            let codes = per.entry(*i).or_default();
            if !codes.contains(code) {
                codes.push(code.clone());
            }
        }
        per.into_iter()
            .map(|(i, mut codes)| {
                codes.sort_by_key(|c| (c.matches('/').count(), c.len()));
                (&self.list[i], codes)
            })
            .collect()
    }

    /// Each dictionary's texts for `code` (the reverse of [`Self::codes_for`]).
    #[must_use]
    pub fn texts_for(&self, code: &str) -> Vec<(&Dictionary, Vec<String>)> {
        let mut per: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (i, text) in self.by_code.get(code).into_iter().flatten() {
            per.entry(*i).or_default().push(text.clone());
        }
        per.into_iter().map(|(i, texts)| (&self.list[i], texts)).collect()
    }

    /// The plain single words of all dictionaries, lowercase: they count as practice words.
    #[must_use]
    pub fn words(&self) -> HashSet<String> {
        self.by_text.keys().filter(|t| !t.contains(char::is_whitespace)).cloned().collect()
    }

    /// The multiword entries' texts, in file order, each once.
    #[must_use]
    pub fn phrases(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.list
            .iter()
            .flat_map(|d| d.entries.iter().map(|(t, _)| t.clone()))
            .filter(|t| t.split_whitespace().count() > 1 && seen.insert(t.clone()))
            .collect()
    }

    /// Word to its typable codes (no whitespace), for accept mode: lowercase single words only.
    #[must_use]
    pub fn accepted_codes(&self) -> HashMap<String, Vec<String>> {
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        for (text, codes) in &self.by_text {
            if text.contains(char::is_whitespace) {
                continue;
            }
            let list = out.entry(text.clone()).or_default();
            for (_, code) in codes {
                if !code.contains(char::is_whitespace) && code != text && !list.contains(code) {
                    list.push(code.clone());
                }
            }
        }
        out.retain(|_, v| !v.is_empty());
        out
    }

    /// The hint text of each of `words` the dictionaries know: ` (CODE)` for one dictionary,
    /// `lyk: CODE, esp: crt` for several, up to two codes of each.
    #[must_use]
    pub fn hints(&self, words: &HashSet<&str>) -> HashMap<String, String> {
        let one = self.list.len() == 1;
        words
            .iter()
            .filter_map(|w| {
                let found = self.codes_for(w);
                if found.is_empty() {
                    return None;
                }
                let parts: Vec<String> = found
                    .iter()
                    .map(|(d, codes)| {
                        let shown = codes.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
                        if one { shown } else { format!("{}: {shown}", d.name) }
                    })
                    .collect();
                Some(((*w).to_owned(), parts.join(", ")))
            })
            .collect()
    }

    /// `typeladder lookup`: what each dictionary has for each of `queries`.
    #[must_use]
    pub fn lookup_text(&self, queries: &[String], dir: &Path) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        if self.list.is_empty() {
            let _ = writeln!(
                out,
                "no dictionaries in {}\nput .json, .yaml or .toml files there (see docs/config.md)",
                dir.display()
            );
        }
        for w in &self.warnings {
            let _ = writeln!(out, "{w}");
        }
        for q in queries {
            let _ = writeln!(out, "{q}");
            let mut any = false;
            for (d, codes) in self.codes_for(q) {
                any = true;
                let _ = writeln!(
                    out,
                    "  {}  [{}, {}]  {}",
                    d.name,
                    d.direction.name(),
                    d.origin.text(),
                    codes.join("  ")
                );
            }
            if !any {
                for (d, texts) in self.texts_for(q) {
                    any = true;
                    let _ = writeln!(
                        out,
                        "  {}  [{}, {}]  is the code of: {}",
                        d.name,
                        d.direction.name(),
                        d.origin.text(),
                        texts.join("  |  ")
                    );
                }
            }
            if !any && !self.list.is_empty() {
                let _ = writeln!(out, "  no dictionary has it");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.join(format!("typeladder-dict-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn load(dir: &Path) -> Dictionaries {
        Dictionaries::load(dir, &BTreeMap::new())
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
    }

    #[test]
    fn the_file_name_sets_the_direction() {
        let dir = scratch("names");
        std::fs::write(dir.join("lyk.codes.json"), r#"{"create": "KRAET", "the": "-T"}"#).unwrap();
        std::fs::write(dir.join("plover.texts.json"), r#"{"KRAET": "create"}"#).unwrap();
        let d = load(&dir);
        assert_eq!(d.list.len(), 2);
        let lyk = &d.list[0];
        assert_eq!(
            (lyk.name.as_str(), lyk.direction, lyk.origin),
            ("lyk", Direction::Codes, Origin::FileName)
        );
        assert_eq!(lyk.entries[0], ("create".to_owned(), "KRAET".to_owned()));
        let plover = &d.list[1];
        assert_eq!((plover.name.as_str(), plover.direction), ("plover", Direction::Texts));
        assert_eq!(
            plover.entries[0],
            ("create".to_owned(), "KRAET".to_owned()),
            "both store (text, code)"
        );
        let found = d.codes_for("create");
        assert_eq!(found.len(), 2, "each dictionary answers under its name");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_beats_the_file_name_and_the_guess() {
        let dir = scratch("config");
        std::fs::write(dir.join("x.codes.json"), r#"{"KRAET": "create"}"#).unwrap();
        let overrides = BTreeMap::from([("x".to_owned(), Direction::Texts)]);
        let d = Dictionaries::load(&dir, &overrides);
        assert_eq!((d.list[0].direction, d.list[0].origin), (Direction::Texts, Origin::Config));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flat_map_is_guessed_by_stroke_shape_then_by_length() {
        // stroke-like values: text to code
        assert_eq!(
            guess(&pairs(&[("create", "KRAET"), ("the", "-T"), ("of", "-F")])),
            (Direction::Codes, false)
        );
        // stroke-like keys: code to text
        assert_eq!(guess(&pairs(&[("KRAET", "create"), ("-T", "the")])), (Direction::Texts, false));
        // espanso-like: short triggers, long replacements
        assert_eq!(
            guess(&pairs(&[("btw", "by the way"), ("sig", "best regards"), ("omw", "on my way")])),
            (Direction::Texts, false)
        );
        // the other way round: long keys, short values
        assert_eq!(
            guess(&pairs(&[("by the way", "btw"), ("on my way", "omw")])),
            (Direction::Codes, false)
        );
        // a tie prefers code to text
        assert_eq!(guess(&pairs(&[("abc", "xyz"), ("hello", "world")])), (Direction::Texts, true));
        assert_eq!(guess(&[]), (Direction::Texts, true));
    }

    #[test]
    fn an_unnamed_file_is_guessed_and_says_so() {
        let dir = scratch("guess");
        std::fs::write(dir.join("short.json"), r#"{"btw": "by the way", "omw": "on my way"}"#).unwrap();
        let d = load(&dir);
        assert_eq!((d.list[0].direction, d.list[0].origin), (Direction::Texts, Origin::Guess(false)));
        let text = d.lookup_text(&["by the way".to_owned()], &dir);
        assert!(text.contains("short  [texts, guessed]  btw"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn espanso_match_files_read_triggers_and_replacements() {
        let dir = scratch("espanso");
        std::fs::write(
            dir.join("base.yml"),
            "name: base\nmatches:\n  - trigger: \":btw\"\n    replace: \"by the way\"\n  - triggers: [\":eml\", \":mail\"]\n    replace: \"me@example.com\"\n  - trigger: \":date\"\n    replace: \"{{d}}\"\n    vars: []\n  - trigger: \":form\"\n    form: \"x\"\n",
        )
        .unwrap();
        let d = load(&dir);
        let e = &d.list[0];
        assert_eq!((e.direction, e.origin), (Direction::Texts, Origin::Format));
        assert_eq!(e.entries.len(), 4, "{:?}", e.entries);
        assert_eq!(d.codes_for("by the way")[0].1, [":btw"]);
        assert_eq!(d.codes_for("me@example.com")[0].1.len(), 2);
        assert_eq!(d.phrases(), ["by the way"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toml_tables_and_record_lists_read_too() {
        let dir = scratch("other");
        std::fs::write(dir.join("t.codes.toml"), "create = \"KRAET\"\n[more]\nthe = \"-T\"\n").unwrap();
        std::fs::write(
            dir.join("r.codes.json"),
            r#"[{"from": "dog", "to": "TKOG"}, {"key": "cat", "value": "KAT"}]"#,
        )
        .unwrap();
        std::fs::write(dir.join("y.texts.yaml"), "KAT: cat\nTKOG:\n  - dog\n  - hound\n").unwrap();
        let d = load(&dir);
        assert_eq!(d.list.len(), 3);
        let by = |n: &str| d.list.iter().find(|x| x.name == n).unwrap();
        assert_eq!(by("t").entries.len(), 2);
        assert_eq!(
            by("r").entries,
            [("dog".to_owned(), "TKOG".to_owned()), ("cat".to_owned(), "KAT".to_owned())]
        );
        assert_eq!(by("y").entries.len(), 3, "a list value is several texts for one code");
        assert_eq!(d.codes_for("hound")[0].1, ["TKOG"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn several_codes_come_fewest_strokes_first_and_lookup_is_case_blind() {
        let dir = scratch("codes");
        std::fs::write(dir.join("lyk.codes.json"), r#"{"create": ["KRAET/-D", "KRAET", "KR*AET"]}"#).unwrap();
        let d = load(&dir);
        assert_eq!(d.codes_for("Create")[0].1, ["KRAET", "KR*AET", "KRAET/-D"]);
        let hints = d.hints(&HashSet::from(["create", "nope"]));
        assert_eq!(hints.get("create").map(String::as_str), Some("KRAET KR*AET"));
        assert!(!hints.contains_key("nope"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hints_name_the_dictionary_when_there_are_several() {
        let dir = scratch("hints");
        std::fs::write(dir.join("lyk.codes.json"), r#"{"create": "KRAET"}"#).unwrap();
        std::fs::write(dir.join("esp.texts.json"), r#"{"crt": "create"}"#).unwrap();
        let d = load(&dir);
        assert_eq!(
            d.hints(&HashSet::from(["create"])).get("create").map(String::as_str),
            Some("esp: crt, lyk: KRAET")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn words_phrases_and_accepted_codes() {
        let dir = scratch("accept");
        std::fs::write(
            dir.join("a.codes.json"),
            r#"{"create": "KRAET", "of the": "-FT", "gizmo": "G/Z M"}"#,
        )
        .unwrap();
        let d = load(&dir);
        assert_eq!(d.words(), HashSet::from(["create".to_owned(), "gizmo".to_owned()]));
        assert_eq!(d.phrases(), ["of the"]);
        let acc = d.accepted_codes();
        assert_eq!(acc.get("create").map(Vec::as_slice), Some(&["KRAET".to_owned()][..]));
        assert!(!acc.contains_key("gizmo"), "a code with a space cannot be typed as one word");
        assert!(!acc.contains_key("of the"), "phrases are not words");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lookup_finds_a_code_too_and_says_when_nothing_has_it() {
        let dir = scratch("lookup");
        std::fs::write(dir.join("lyk.codes.json"), r#"{"create": "KRAET"}"#).unwrap();
        let d = load(&dir);
        let text = d.lookup_text(&["create".into(), "KRAET".into(), "zzz".into()], &dir);
        assert!(text.contains("lyk  [codes, from the file name]  KRAET"), "{text}");
        assert!(text.contains("is the code of: create"), "{text}");
        assert!(text.contains("no dictionary has it"), "{text}");
        let none = Dictionaries::load(&dir.join("nowhere"), &BTreeMap::new());
        assert!(none.list.is_empty() && none.lookup_text(&["x".into()], &dir).contains("no dictionaries"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_file_is_a_warning_and_the_rest_still_load() {
        let dir = scratch("broken");
        std::fs::write(dir.join("bad.json"), "{not json").unwrap();
        std::fs::write(dir.join("ok.codes.json"), r#"{"a": "b"}"#).unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let d = load(&dir);
        assert_eq!(d.list.len(), 1);
        assert_eq!(d.warnings.len(), 1);
        assert!(d.warnings[0].contains("bad.json"), "{:?}", d.warnings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stroke_shapes() {
        for s in ["KRAET", "-T", "STKPW*", "KRAET/-D", "#S", "TPH-PB"] {
            assert!(stroke_like(s), "{s}");
        }
        for s in ["create", "Create", "by the way", ":btw", "", "-"] {
            assert!(!stroke_like(s), "{s}");
        }
    }
}
