//! `fetch`: download Monkeytype's English word lists into the data folder.
//!
//! The lists are Monkeytype's (GPL-3.0). Files that are already there are skipped, so it is safe
//! to run again. The first run of a practice test fetches the missing small lists by itself
//! ([`ensure_lists`]), once a day at most when the network is down.

use anyhow::{Context, Result};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const MONKEYTYPE: &str = "https://raw.githubusercontent.com/monkeytypegame/monkeytype/master/frontend/static/languages";

/// Monkeytype's English word lists (`{name, words[]}` JSON). The first five are ordered by
/// frequency (`english_25k` is not); `english_450k` is an unordered dictionary.
pub const MONKEYTYPE_LISTS: [&str; 6] = ["english", "english_1k", "english_5k", "english_10k", "english_25k", "english_450k"];
/// The lists the first run fetches by itself (the 450k dictionary is 8 MB, only for `--deep`).
const FIRST_RUN: [&str; 5] = ["english", "english_1k", "english_5k", "english_10k", "english_25k"];
/// After a failed automatic download the next attempt waits this long.
const RETRY_AFTER: Duration = Duration::from_hours(24);

fn target(data: &Path, name: &str) -> PathBuf {
    data.join("wordlists").join(format!("monkeytype-{name}.json"))
}

fn have(data: &Path, name: &str) -> bool {
    fs::metadata(target(data, name)).is_ok_and(|m| m.len() > 0)
}

/// Download to a temporary name and rename, so a failed download never leaves half a file.
fn download(url: &str, target: &Path) -> Result<()> {
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir)?;
    }
    let part = target.with_extension("part");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(60))
        .build();
    let response = agent.get(url).call()?;
    let mut file = fs::File::create(&part)?;
    io::copy(&mut response.into_reader(), &mut file)?;
    fs::rename(&part, target)?;
    Ok(())
}

/// `typeladder fetch`: every list not on disk yet.
pub fn run(data: &Path) -> Result<()> {
    for name in MONKEYTYPE_LISTS {
        let path = format!("wordlists/monkeytype-{name}.json");
        if have(data, name) {
            println!("have  {path}");
            continue;
        }
        println!("fetch {path}");
        let url = format!("{MONKEYTYPE}/{name}.json");
        download(&url, &target(data, name)).with_context(|| format!("{url} -> {path}"))?;
    }
    let _ = fs::remove_file(data.join(".fetch-failed"));
    Ok(())
}

/// The lists a first run is missing: `deep` adds the 450k dictionary.
#[must_use]
pub fn missing(data: &Path, deep: bool) -> Vec<&'static str> {
    FIRST_RUN
        .iter()
        .copied()
        .chain(deep.then_some("english_450k"))
        .filter(|n| !have(data, n))
        .collect()
}

/// Downloads the missing lists before a test, saying so; on a failure says what the test uses
/// instead and does not try again for a day. `lines` receives each message.
pub fn ensure_lists(data: &Path, deep: bool, lines: &mut dyn FnMut(String)) {
    // the built-in 200 words stand in for `english`, so it is not worth a download on its own
    let wanted: Vec<&str> = missing(data, deep).into_iter().filter(|n| *n != "english").collect();
    if wanted.is_empty() {
        return;
    }
    let marker = data.join(".fetch-failed");
    if fs::metadata(&marker)
        .and_then(|m| m.modified())
        .is_ok_and(|t| SystemTime::now().duration_since(t).is_ok_and(|age| age < RETRY_AFTER))
    {
        return;
    }
    lines(format!(
        "first run: downloading Monkeytype's word lists ({}) to {}",
        wanted.join(", "),
        data.join("wordlists").display()
    ));
    for name in &wanted {
        if let Err(e) = download(&format!("{MONKEYTYPE}/{name}.json"), &target(data, name)) {
            let _ = fs::create_dir_all(data);
            let _ = fs::write(&marker, e.to_string());
            lines(format!(
                "could not download {name}: {e}; practising with what is on disk (at least the built-in 200 words); `typeladder fetch` tries again"
            ));
            return;
        }
    }
    let _ = fs::remove_file(&marker);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.join(format!("typeladder-fetch-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn what_is_missing_depends_on_the_disk_and_deep() {
        let dir = scratch("missing");
        assert_eq!(
            missing(&dir, false),
            ["english", "english_1k", "english_5k", "english_10k", "english_25k"]
        );
        fs::create_dir_all(dir.join("wordlists")).unwrap();
        fs::write(target(&dir, "english_1k"), "{}").unwrap();
        fs::write(target(&dir, "english_5k"), "").unwrap();
        let m = missing(&dir, true);
        assert!(
            !m.contains(&"english_1k") && m.contains(&"english_5k") && m.contains(&"english_450k"),
            "{m:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recent_failure_is_not_retried_and_nothing_is_said() {
        let dir = scratch("marker");
        fs::write(dir.join(".fetch-failed"), "offline").unwrap();
        let mut said = Vec::new();
        ensure_lists(&dir, false, &mut |l| said.push(l));
        assert_eq!(said.len(), 0, "{said:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_is_said_when_every_list_is_there() {
        let dir = scratch("all");
        fs::create_dir_all(dir.join("wordlists")).unwrap();
        for n in FIRST_RUN {
            fs::write(target(&dir, n), "{}").unwrap();
        }
        let mut said = Vec::new();
        ensure_lists(&dir, false, &mut |l| said.push(l));
        assert_eq!(said.len(), 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
