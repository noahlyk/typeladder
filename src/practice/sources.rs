//! Where the units of a fixed feed come from. Each loader returns plain units, never hints.

use super::feed::is_plain_word;
use super::pool::Candidates;
use anyhow::{Context, Result, bail};
use std::path::Path;

/// The commonest words, in rank order: the first `reach` words of the pool.
pub fn common(pool: &mut dyn Candidates, reach: usize) -> Result<Vec<String>> {
    let words: Vec<String> = (0..reach).map_while(|i| pool.get(i).map(str::to_owned)).collect();
    if words.is_empty() {
        bail!("the word pool is empty: run `typeladder fetch` to download the Monkeytype lists (`typeladder where` shows the folder)");
    }
    Ok(words)
}

/// Words of a plain text file, split on whitespace with the punctuation around each trimmed.
pub fn file(path: &Path, names: bool) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').replace('\u{2019}', "'"))
        .filter(|w| is_plain_word(w, names))
        .collect();
    if words.is_empty() {
        bail!("{}: no plain words in it", path.display());
    }
    Ok(words)
}

/// The phrases of the dictionaries (their multiword entries), each once, as the text they type.
/// Only phrases of plain words qualify.
pub fn phrases(all: Vec<String>) -> Result<Vec<String>> {
    let mut seen = std::collections::HashSet::new();
    let phrases: Vec<String> = all
        .into_iter()
        .filter(|p| p.split(' ').all(|w| is_plain_word(w, false)) && seen.insert(p.clone()))
        .collect();
    if phrases.is_empty() {
        bail!("no phrases: put a dictionary with multiword entries in the dictionaries folder (`typeladder where` shows it)");
    }
    Ok(phrases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_reads_the_first_words_of_the_pool_in_order() {
        let mut pool: Vec<String> = ["the", "of", "and", "to"].map(String::from).to_vec();
        assert_eq!(common(&mut pool, 3).unwrap(), ["the", "of", "and"]);
        assert_eq!(common(&mut pool, 10).unwrap().len(), 4, "only what there is");
        assert!(common(&mut Vec::<String>::new(), 3).is_err());
    }

    #[test]
    fn a_text_file_gives_its_plain_words() {
        let dir = std::env::temp_dir().join(format!("typeladder-sources-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.txt");
        std::fs::write(&path, "Hello, world! it\u{2019}s \"fine\" 42.").unwrap();
        assert_eq!(file(&path, false).unwrap(), ["world", "it's", "fine"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn phrases_are_unique_and_plain() {
        let got = phrases(vec!["of the".into(), "of the".into(), "a-b c".into(), "in a".into()]).unwrap();
        assert_eq!(got, ["of the", "in a"]);
        assert!(phrases(vec!["x y z1".into()]).is_err());
    }
}
