//! Where the config, the dictionaries, the word lists and the records live, so the same binary
//! works from a checkout and from anywhere once installed.
//!
//! Three folders, each resolved the same way by every command, first match wins:
//!
//! | | config (yours, hand-edited) | data (downloaded word lists) | state (written by the program) |
//! |---|---|---|---|
//! | 1. env | `TYPELADDER_CONFIG_DIR` | `TYPELADDER_DATA_DIR` | `TYPELADDER_STATE_DIR` |
//! | 2. current folder is a checkout | `config` | `data` | `state` |
//! | 3. XDG | `$XDG_CONFIG_HOME/typeladder` | `$XDG_DATA_HOME/typeladder` | `$XDG_STATE_HOME/typeladder` |
//!
//! The XDG homes default to `~/.config`, `~/.local/share` and `~/.local/state`; with no `$HOME`
//! either, the relative `config`, `data` and `state` are used. A checkout is a folder whose
//! `Cargo.toml` names this package and which has `src/`.
//!
//! Inside config: `config.toml` and `dictionaries/`. Inside data: `wordlists/`. Inside state:
//! `vocab.json` and `history.jsonl`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What decides the folders: the environment and whether the current folder is a checkout.
#[derive(Debug, Default, Clone)]
pub struct Env {
    pub config_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub checkout: bool,
}

/// Which rule picked a folder, for `where`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    Checkout,
    Xdg,
    Fallback,
}

impl Source {
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Env => "environment",
            Self::Checkout => "current folder is a checkout",
            Self::Xdg => "XDG default",
            Self::Fallback => "no $HOME: relative",
        }
    }
}

fn non_empty(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Whether `dir` is a checkout of this repository.
#[must_use]
pub fn is_checkout(dir: &Path) -> bool {
    let named = std::fs::read_to_string(dir.join("Cargo.toml"))
        .is_ok_and(|t| t.lines().any(|l| l.trim().replace(' ', "") == "name=\"typeladder\""));
    named && dir.join("src").is_dir()
}

impl Env {
    /// The real environment and current folder.
    #[must_use]
    pub fn current() -> Self {
        Self {
            config_dir: non_empty("TYPELADDER_CONFIG_DIR"),
            data_dir: non_empty("TYPELADDER_DATA_DIR"),
            state_dir: non_empty("TYPELADDER_STATE_DIR"),
            xdg_config_home: non_empty("XDG_CONFIG_HOME").filter(|p| p.is_absolute()),
            xdg_data_home: non_empty("XDG_DATA_HOME").filter(|p| p.is_absolute()),
            xdg_state_home: non_empty("XDG_STATE_HOME").filter(|p| p.is_absolute()),
            home: non_empty("HOME"),
            checkout: is_checkout(Path::new(".")),
        }
    }

    fn pick(&self, env: Option<&PathBuf>, xdg_home: Option<&PathBuf>, under_home: &str, leaf: &str) -> (PathBuf, Source) {
        if let Some(env) = env {
            return (env.clone(), Source::Env);
        }
        if self.checkout {
            return (PathBuf::from(leaf), Source::Checkout);
        }
        xdg_home
            .cloned()
            .or_else(|| self.home.as_ref().map(|h| h.join(under_home)))
            .map_or_else(
                || (PathBuf::from(leaf), Source::Fallback),
                |base| (base.join("typeladder"), Source::Xdg),
            )
    }

    #[must_use]
    pub fn config(&self) -> (PathBuf, Source) {
        self.pick(self.config_dir.as_ref(), self.xdg_config_home.as_ref(), ".config", "config")
    }

    #[must_use]
    pub fn data(&self) -> (PathBuf, Source) {
        self.pick(self.data_dir.as_ref(), self.xdg_data_home.as_ref(), ".local/share", "data")
    }

    #[must_use]
    pub fn state(&self) -> (PathBuf, Source) {
        self.pick(self.state_dir.as_ref(), self.xdg_state_home.as_ref(), ".local/state", "state")
    }
}

fn env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(Env::current)
}

#[must_use]
pub fn config_dir() -> PathBuf {
    env().config().0
}

#[must_use]
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

#[must_use]
pub fn dictionaries_dir() -> PathBuf {
    config_dir().join("dictionaries")
}

#[must_use]
pub fn data_dir() -> PathBuf {
    env().data().0
}

#[must_use]
pub fn state_dir() -> PathBuf {
    env().state().0
}

/// What the adaptive source knows about each word.
#[must_use]
pub fn vocab_file() -> PathBuf {
    state_dir().join("vocab.json")
}

/// One JSON line per finished test.
#[must_use]
pub fn history_file() -> PathBuf {
    state_dir().join("history.jsonl")
}

/// `typeladder where`: each folder, what decided it, and the files in it.
#[must_use]
pub fn where_text() -> String {
    let e = env();
    let rows = [
        ("config", e.config().1, config_dir()),
        ("dictionaries", e.config().1, dictionaries_dir()),
        ("data", e.data().1, data_dir()),
        ("state", e.state().1, state_dir()),
    ];
    let mut out = String::new();
    for (name, source, path) in rows {
        let _ = writeln!(out, "{name:<13}{}  ({})", path.display(), source.text());
    }
    let _ = writeln!(out, "{:<13}{}", "config file", config_file().display());
    let _ = writeln!(out, "{:<13}{}", "vocab", vocab_file().display());
    let _ = writeln!(out, "{:<13}{}", "history", history_file().display());
    out
}

/// A leading `~` as the home folder.
#[must_use]
pub fn expand_home(path: &Path) -> PathBuf {
    match (path.strip_prefix("~"), std::env::var_os("HOME")) {
        (Ok(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(f: impl FnOnce(&mut Env)) -> Env {
        let mut e = Env {
            home: Some("/home/u".into()),
            ..Env::default()
        };
        f(&mut e);
        e
    }

    #[test]
    fn xdg_defaults_live_under_the_home_folder() {
        let e = env(|_| {});
        assert_eq!(e.config(), (PathBuf::from("/home/u/.config/typeladder"), Source::Xdg));
        assert_eq!(e.data(), (PathBuf::from("/home/u/.local/share/typeladder"), Source::Xdg));
        assert_eq!(e.state(), (PathBuf::from("/home/u/.local/state/typeladder"), Source::Xdg));
    }

    #[test]
    fn xdg_homes_move_the_folders() {
        let e = env(|e| {
            e.xdg_config_home = Some("/c".into());
            e.xdg_data_home = Some("/d".into());
            e.xdg_state_home = Some("/s".into());
        });
        assert_eq!(e.config().0, PathBuf::from("/c/typeladder"));
        assert_eq!(e.data().0, PathBuf::from("/d/typeladder"));
        assert_eq!(e.state().0, PathBuf::from("/s/typeladder"));
    }

    #[test]
    fn the_environment_beats_a_checkout_and_a_checkout_beats_xdg() {
        let e = env(|e| {
            e.checkout = true;
            e.data_dir = Some("/mine".into());
        });
        assert_eq!(e.data(), (PathBuf::from("/mine"), Source::Env));
        assert_eq!(e.state(), (PathBuf::from("state"), Source::Checkout));
        assert_eq!(e.config(), (PathBuf::from("config"), Source::Checkout));
    }

    #[test]
    fn with_no_home_the_folders_are_relative() {
        let e = Env::default();
        assert_eq!(e.state(), (PathBuf::from("state"), Source::Fallback));
    }

    #[test]
    fn a_leading_tilde_is_the_home_folder() {
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(expand_home(Path::new("~/x")), PathBuf::from(home).join("x"));
        }
        assert_eq!(expand_home(Path::new("/a/b")), PathBuf::from("/a/b"));
    }
}
