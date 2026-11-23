//! `typeladder import --from-lyk`: bring the vocabulary and the run history of the lyk steno
//! theory's `lyk practice` over, so the adaptive test carries on where it was.
//!
//! The source folder is never changed. An existing typeladder file is never overwritten unless
//! `--force`, which first keeps it as `NAME.bak`.

use crate::paths;
use crate::vocab::Vocab;
use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use std::path::{Path, PathBuf};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Import lyk's `vocab.json` and `practice.jsonl`
    #[arg(long)]
    from_lyk: bool,
    /// Say what would be copied, write nothing
    #[arg(long)]
    dry_run: bool,
    /// lyk's state folder [default: `$XDG_STATE_HOME/lyk`, else `~/.local/state/lyk`]
    #[arg(long, value_name = "DIR")]
    lyk_state: Option<PathBuf>,
    /// Overwrite files that exist here already (each is kept as `NAME.bak` first)
    #[arg(long)]
    force: bool,
}

/// lyk's state folder.
fn lyk_state_dir() -> PathBuf {
    let xdg = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    xdg.or_else(|| {
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(|h| PathBuf::from(h).join(".local/state"))
    })
    .unwrap_or_default()
    .join("lyk")
}

pub fn run(args: &Args) -> Result<()> {
    if !args.from_lyk {
        bail!("nothing to import: say from where (`typeladder import --from-lyk`)");
    }
    let src = args.lyk_state.clone().unwrap_or_else(lyk_state_dir);
    print!("{}", import_lyk(&src, &paths::state_dir(), args.dry_run, args.force)?);
    Ok(())
}

/// What a file is worth saying about: its name here and there and a summary of what is in it.
struct Item {
    from: PathBuf,
    to: PathBuf,
    name: &'static str,
    summary: String,
}

fn summarise_vocab(path: &Path) -> Result<String> {
    let v = Vocab::load(path)?;
    Ok(format!(
        "{} words ({} learning, {} review, {} retired), {} tests",
        v.words.len(),
        v.count(crate::vocab::Status::Learning),
        v.count(crate::vocab::Status::Review),
        v.count(crate::vocab::Status::Retired),
        v.sessions
    ))
}

fn summarise_history(path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let (lines, ok) = text.lines().filter(|l| !l.trim().is_empty()).fold((0, 0), |(n, ok), l| {
        (n + 1, ok + usize::from(serde_json::from_str::<serde_json::Value>(l).is_ok()))
    });
    Ok(format!(
        "{ok} runs{}",
        if ok == lines {
            String::new()
        } else {
            format!(" ({} unreadable lines are copied as they are)", lines - ok)
        }
    ))
}

/// Copies what `src` (lyk's state folder) has into `dest` (typeladder's); the text says what was
/// or, with `dry_run`, would be done. Nothing is copied if any destination file exists without
/// `force`.
pub fn import_lyk(src: &Path, dest: &Path, dry_run: bool, force: bool) -> Result<String> {
    use std::fmt::Write as _;
    if !src.is_dir() {
        bail!("{}: no lyk state folder (use --lyk-state DIR)", src.display());
    }
    let mut items = Vec::new();
    let mut out = String::new();
    for (from_name, name, summarise) in [
        ("vocab.json", "vocab.json", summarise_vocab as fn(&Path) -> Result<String>),
        ("practice.jsonl", "history.jsonl", summarise_history),
    ] {
        let from = src.join(from_name);
        if !from.is_file() {
            let _ = writeln!(out, "skip  {from_name}: not in {}", src.display());
            continue;
        }
        let summary = summarise(&from)?;
        items.push(Item {
            from,
            to: dest.join(name),
            name,
            summary,
        });
    }
    if items.is_empty() {
        bail!("{}: neither vocab.json nor practice.jsonl is there", src.display());
    }
    let clashes: Vec<String> = items.iter().filter(|i| i.to.exists()).map(|i| i.to.display().to_string()).collect();
    if !clashes.is_empty() && !force {
        bail!(
            "{} exists already; nothing was copied (--force keeps it as .bak and overwrites, --dry-run shows the plan)",
            clashes.join(" and ")
        );
    }
    for i in &items {
        let kept = i.to.exists();
        let _ = writeln!(
            out,
            "{}  {} -> {}: {}{}",
            if dry_run { "would copy" } else { "copied" },
            i.from.display(),
            i.to.display(),
            i.summary,
            if kept { "; the old one is kept as .bak" } else { "" }
        );
        if dry_run {
            continue;
        }
        std::fs::create_dir_all(dest).with_context(|| dest.display().to_string())?;
        if kept {
            let bak = dest.join(format!("{}.bak", i.name));
            std::fs::copy(&i.to, &bak).with_context(|| format!("cannot back up {} to {}", i.to.display(), bak.display()))?;
        }
        let part = dest.join(format!("{}.part", i.name));
        std::fs::copy(&i.from, &part).with_context(|| format!("cannot copy {}", i.from.display()))?;
        std::fs::rename(&part, &i.to).with_context(|| i.to.display().to_string())?;
    }
    if !dry_run {
        out.push_str("done; the source folder was not changed. Start with `typeladder`.\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.join(format!("typeladder-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A vocab.json as lyk writes it, with every optional field it has ever had.
    const LYK_VOCAB: &str = r#"{"version":1,"seeded":true,"sessions":12,"recent":"1101","base_recent":"111","base_tests":[[9,10]],"gate":0.8,"last_new":19900,"filter_version":1,"hidden":["er"],"goal":"lyk","words":{"the":{"status":"review","attempts":40,"hits":38,"recent":"1111111111","speed_ms":420.5,"last_seen":19990,"due":20010,"level":3,"days":9,"promotions":3,"lapses":0,"streak":0,"mastered":19950,"stumbles":2,"stumbled_in":0,"stumbled_n":0,"practiced":30,"learn_n":12,"learn_hits":11},"cat":{"status":"learning","attempts":5,"hits":3,"stuck":true}}}"#;
    const LYK_HISTORY: &str = "{\"timestamp\":1,\"source\":\"adaptive\",\"count\":25,\"wpm\":50.0,\"raw\":52.0,\"accuracy\":97.0,\"errors\":1,\"secs\":30.0,\"missed\":[\"the\"],\"stumbled\":[],\"complete\":true}\nnot json\n";

    fn lyk_dir(name: &str) -> PathBuf {
        let dir = scratch(name).join("lyk");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vocab.json"), LYK_VOCAB).unwrap();
        std::fs::write(dir.join("practice.jsonl"), LYK_HISTORY).unwrap();
        dir
    }

    #[test]
    fn a_lyk_vocab_loads_unchanged_after_the_import() {
        let src = lyk_dir("copy");
        let dest = src.parent().unwrap().join("typeladder");
        let text = import_lyk(&src, &dest, false, false).unwrap();
        assert!(
            text.contains("2 words (1 learning, 1 review, 0 retired), 12 tests")
                && text.contains("1 runs")
                && text.contains("1 unreadable"),
            "{text}"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("vocab.json")).unwrap(),
            LYK_VOCAB,
            "copied byte for byte"
        );
        assert_eq!(std::fs::read_to_string(dest.join("history.jsonl")).unwrap(), LYK_HISTORY);
        let v = Vocab::load(&dest.join("vocab.json")).unwrap();
        assert_eq!(
            (v.words["the"].level, v.words["the"].mastered, v.goal.as_deref(), v.gate),
            (3, Some(19950), Some("lyk"), Some(0.8))
        );
        assert!(v.words["cat"].stuck && v.hidden.contains("er"));
        assert_eq!(
            std::fs::read_to_string(src.join("vocab.json")).unwrap(),
            LYK_VOCAB,
            "the source is untouched"
        );
        assert!(!dest.join("vocab.json.part").exists());
        let _ = std::fs::remove_dir_all(src.parent().unwrap());
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let src = lyk_dir("dry");
        let dest = src.parent().unwrap().join("typeladder");
        let text = import_lyk(&src, &dest, true, false).unwrap();
        assert!(text.contains("would copy") && !text.contains("done"), "{text}");
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(src.parent().unwrap());
    }

    #[test]
    fn an_existing_file_is_refused_unless_forced_and_then_backed_up() {
        let src = lyk_dir("force");
        let dest = src.parent().unwrap().join("typeladder");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("vocab.json"), r#"{"version":1,"seeded":true,"words":{}}"#).unwrap();
        let err = import_lyk(&src, &dest, false, false).unwrap_err().to_string();
        assert!(err.contains("exists already") && err.contains("nothing was copied"), "{err}");
        assert!(
            !dest.join("history.jsonl").exists(),
            "all or nothing: the history was not copied either"
        );
        // a dry run with a clash still refuses to promise anything
        assert!(import_lyk(&src, &dest, true, false).is_err());
        import_lyk(&src, &dest, false, true).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("vocab.json")).unwrap(), LYK_VOCAB);
        assert_eq!(
            std::fs::read_to_string(dest.join("vocab.json.bak")).unwrap(),
            r#"{"version":1,"seeded":true,"words":{}}"#
        );
        assert!(!dest.join("history.jsonl.bak").exists(), "nothing to keep there");
        let _ = std::fs::remove_dir_all(src.parent().unwrap());
    }

    #[test]
    fn a_missing_folder_or_files_say_so_and_a_bad_vocab_is_refused() {
        let dir = scratch("missing");
        assert!(
            import_lyk(&dir.join("nope"), &dir.join("d"), false, false)
                .unwrap_err()
                .to_string()
                .contains("no lyk state folder")
        );
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(import_lyk(&empty, &dir.join("d"), false, false).is_err());
        std::fs::write(empty.join("vocab.json"), "{broken").unwrap();
        assert!(import_lyk(&empty, &dir.join("d"), false, false).is_err());
        assert!(!dir.join("d").exists());
        // only the history there: the vocab is skipped
        std::fs::remove_file(empty.join("vocab.json")).unwrap();
        std::fs::write(empty.join("practice.jsonl"), LYK_HISTORY).unwrap();
        let text = import_lyk(&empty, &dir.join("d"), false, false).unwrap();
        assert!(text.contains("skip  vocab.json") && dir.join("d/history.jsonl").exists(), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
