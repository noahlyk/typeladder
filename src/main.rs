//! typeladder: an adaptive typing test that builds your vocabulary from the commonest word to the
//! rarest, re-drills what you miss, retires what you know and brings it back for review.
//!
//! It reads ordinary key presses in a terminal, so a steno engine, a text expander and a plain
//! keyboard all work the same and the program never knows which. Your own dictionaries (shortcut
//! or outline tables) are optional: they give hints, phrases and extra accepted answers.

// Long functions are fine here: the screen, the planner and the commands read top to bottom.
#![allow(clippy::too_many_lines)]

mod config;
mod dict;
mod fetch;
mod hooks;
mod import;
mod paths;
mod practice;
mod vocab;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::io::IsTerminal;

/// The version `--version` prints, baked in by `build.rs`.
const VERSION: &str = env!("TYPELADDER_VERSION");

#[derive(Parser)]
#[command(name = "typeladder", version = VERSION, about, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(flatten)]
    practice: practice::Args,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// The adaptive typing test (what `typeladder` alone does)
    Practice(practice::Args),
    /// What the test knows about each word: the summary, and the commands that change it
    Vocab(vocab::Args),
    /// Show a word's codes in your dictionaries (or what a code stands for)
    Lookup {
        /// Words or phrases (or codes)
        #[arg(required = true)]
        words: Vec<String>,
    },
    /// Download Monkeytype's English word lists
    Fetch,
    /// Show the folders in use and what decided each
    Where,
    /// Bring your history over from another program
    Import(import::Args),
    /// Print the version (the same as `--version`)
    #[command(hide = true)]
    Version,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("typeladder: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => practice(&cli.practice),
        Some(Command::Practice(args)) => practice(&args),
        Some(Command::Vocab(args)) => {
            let cfg = config::Config::load(&paths::config_file())?;
            let dicts = dict::Dictionaries::load(&paths::dictionaries_dir(), &cfg.directions());
            vocab::run(&args, &cfg, &dicts)
        }
        Some(Command::Lookup { words }) => {
            let cfg = config::Config::load(&paths::config_file())?;
            let dir = paths::dictionaries_dir();
            print!("{}", dict::Dictionaries::load(&dir, &cfg.directions()).lookup_text(&words, &dir));
            Ok(())
        }
        Some(Command::Fetch) => fetch::run(&paths::data_dir()),
        Some(Command::Where) => {
            print!("{}", paths::where_text());
            Ok(())
        }
        Some(Command::Import(args)) => import::run(&args),
        Some(Command::Version) => {
            println!("typeladder {VERSION}");
            Ok(())
        }
    }
}

/// The practice test: config, dictionaries, the first-run download and the hooks around it.
fn practice(args: &practice::Args) -> Result<()> {
    let cfg = config::Config::load(&paths::config_file())?;
    let dicts = dict::Dictionaries::load(&paths::dictionaries_dir(), &cfg.directions());
    let interactive = !args.history() && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let hooks = hooks::Hooks::new(cfg.hooks.clone(), interactive && !args.no_hooks());
    if interactive {
        for w in &dicts.warnings {
            eprintln!("{w}");
        }
        if cfg.auto_fetch && !args.no_fetch() && args.needs_lists() {
            fetch::ensure_lists(&paths::data_dir(), args.deep(), &mut |l| eprintln!("{l}"));
        }
    }
    let _session = hooks.start_session();
    practice::run(args, &practice::Ctx { cfg, dicts, hooks })
}
