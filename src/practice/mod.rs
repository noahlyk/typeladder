//! `typeladder practice`: a monkeytype-style word feed in the terminal.
//!
//! A steno engine, a text expander or a plain keyboard all type finished words into the terminal
//! as ordinary characters, so this reads plain key presses and never knows which. The screen shows
//! the words and how the typing is going, nothing else: no codes, no keyboard, no hints. `--hints`
//! adds the dictionary codes of each missed or slow word to the results only.

// Counts and seconds become percentages and rates: the casts are exact at these sizes.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

pub mod adaptive;
mod engine;
mod feed;
mod history;
pub mod pool;
mod report;
mod screen;
mod sources;
pub mod tiers;
mod ui;
pub mod words;

use crate::config::{Accept, Config};
use crate::dict::Dictionaries;
use crate::hooks::{Event as HookEvent, Hooks};
use crate::paths;
use crate::vocab::{self, Vocab};
use anyhow::{Context, Result, bail};
use clap::{Args as ClapArgs, ValueEnum};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use engine::{Input, Session};
use ratatui::DefaultTerminal;
use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq, Debug)]
pub enum Source {
    /// Words you are learning, due reviews and new words you have earned (the default)
    Adaptive,
    /// The commonest words (see --rank, --bands)
    Common,
    /// The words of a text file (--file)
    File,
    /// The multiword entries of your dictionaries, typed as their text
    Phrases,
}

impl Source {
    const fn name(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::Common => "common",
            Self::File => "file",
            Self::Phrases => "phrases",
        }
    }
}

#[derive(ClapArgs, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct Args {
    /// Where the words come from [default: adaptive; file when --file is given]
    #[arg(long, value_enum)]
    source: Option<Source>,
    /// Adaptive: new words entering in one test
    #[arg(long, default_value_t = 2)]
    new_per_session: usize,
    /// Adaptive: most words being learned at once
    #[arg(long, default_value_t = 10)]
    learning_cap: usize,
    /// Adaptive: clean share (percent) of the easy base (first attempts of review words, last 50) that earns new words [default: 70, or what `typeladder vocab gate` set]
    #[arg(long, value_name = "PERCENT")]
    gate: Option<f64>,
    /// Adaptive: also practise the rest of Monkeytype's 450k dictionary (rarer real words; fragments, names and noise stay out)
    #[arg(long)]
    deep: bool,
    /// Adaptive: play the feed without updating vocab.json
    #[arg(long)]
    no_adapt: bool,
    /// The text file for `--source file`
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,
    /// Rank bands of `common`, such as 1-200,201-1000 (1 is the commonest word of the pool)
    #[arg(long, value_name = "BANDS", conflicts_with = "rank")]
    bands: Option<String>,
    /// One rank band of `common`, such as 1-1000 (the default)
    #[arg(long, value_name = "MIN-MAX")]
    rank: Option<String>,
    /// Words in the run
    #[arg(long, default_value_t = 25, conflicts_with = "time")]
    count: usize,
    /// Run for this many seconds instead of a word count
    #[arg(long, value_name = "SECS")]
    time: Option<u64>,
    /// Seed for a repeatable feed
    #[arg(long)]
    seed: Option<u64>,
    /// Draw every band equally often, not every word equally
    #[arg(long)]
    weighted: bool,
    /// Use each word or phrase once
    #[arg(long)]
    no_repeat: bool,
    /// Drop any word with a capital
    #[arg(long)]
    lowercase_only: bool,
    /// Shortest word, in letters
    #[arg(long, default_value_t = 1)]
    min_len: usize,
    /// Longest word, in letters
    #[arg(long, default_value_t = 30)]
    max_len: usize,
    /// Allow capitalised words (names) where they are normally dropped
    #[arg(long)]
    names: bool,
    /// A wrong word is not accepted until it is right
    #[arg(long)]
    stop_on_error: bool,
    /// Adaptive: a word that stumbled or was typed wrong comes back 3 words later in place of a base word (the feed is fixed at the start by default)
    #[arg(long)]
    requeue: bool,
    /// No live speed and accuracy line and no key hint
    #[arg(long)]
    quiet: bool,
    /// Show the dictionary codes of each missed or slow word on the results (also `hints = true` in config.toml)
    #[arg(long)]
    hints: bool,
    /// Include the dictionaries' multiword entries as practice items (also `phrases = true` in config.toml); in the adaptive source they are not tracked in the vocabulary
    #[arg(long)]
    phrases: bool,
    /// What counts as a right answer: `text`, or `text,code` to accept a word's dictionary codes too (also `accept` in config.toml)
    #[arg(long, value_enum, value_delimiter = ',')]
    accept: Option<Vec<Accept>>,
    /// Do not run the hooks of config.toml
    #[arg(long)]
    no_hooks: bool,
    /// Do not download Monkeytype's word lists when they are missing (also `auto_fetch = false`)
    #[arg(long)]
    no_fetch: bool,
    /// Do not record the run in the history
    #[arg(long)]
    no_save: bool,
    /// Print the recent runs and the trend, then exit
    #[arg(long)]
    history: bool,
}

impl Args {
    pub const fn history(&self) -> bool {
        self.history
    }

    pub const fn no_hooks(&self) -> bool {
        self.no_hooks
    }

    pub const fn no_fetch(&self) -> bool {
        self.no_fetch
    }

    pub const fn deep(&self) -> bool {
        self.deep
    }

    /// Whether the source reads Monkeytype's lists.
    pub fn needs_lists(&self) -> bool {
        matches!(
            self.source
                .unwrap_or_else(|| if self.file.is_some() { Source::File } else { Source::Adaptive }),
            Source::Adaptive | Source::Common
        )
    }
}

/// A timed run's feed is sized for this many words a second at most.
const TIMED_WORDS_PER_SECOND: u64 = 8;
const RECENT_RUNS: usize = 15;

/// What a run needs besides its arguments: the config, the dictionaries and the hooks.
pub struct Ctx {
    pub cfg: Config,
    pub dicts: Dictionaries,
    pub hooks: Hooks,
}

impl Ctx {
    /// Whether the results show hints.
    const fn hints(&self, args: &Args) -> bool {
        args.hints || self.cfg.hints
    }

    const fn phrases(&self, args: &Args) -> bool {
        args.phrases || self.cfg.phrases
    }

    fn accepts_code(&self, args: &Args) -> bool {
        args.accept
            .as_ref()
            .map_or_else(|| self.cfg.accepts_code(), |a| a.contains(&Accept::Code))
    }
}

pub fn run(args: &Args, ctx: &Ctx) -> Result<()> {
    if args.history {
        print!("{}", history::report(&history::read(&paths::history_file()), RECENT_RUNS));
        if let Some(line) = stuck_line(Vocab::load(&vocab::file()).map_or(0, |v| v.stuck_count())) {
            println!("{line}");
        }
        return Ok(());
    }
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        bail!(
            "typeladder practice needs a terminal: stdin and stdout must both be a TTY (run it in a terminal, not through a pipe or redirect); `typeladder practice --history` works anywhere"
        );
    }
    let source = args
        .source
        .unwrap_or_else(|| if args.file.is_some() { Source::File } else { Source::Adaptive });
    let phrases = if ctx.phrases(args) { ctx.dicts.phrases() } else { Vec::new() };
    let mut feeder = if source == Source::Adaptive {
        Feeder::Adaptive(Box::new(Adaptive::open(args, ctx)?))
    } else {
        Feeder::Fixed(make_pool(args, ctx, source, &phrases)?)
    };
    let codes = if ctx.accepts_code(args) {
        ctx.dicts.accepted_codes()
    } else {
        HashMap::new()
    };
    if feeder
        .words(args, next_seed(args.seed, 0, random_seed()), &phrases)
        .words
        .is_empty()
    {
        bail!("the filters leave no words to practise: loosen --min-len, --max-len, --lowercase-only or --rank");
    }
    let limit = args.time.map(|t| t as f64);
    let mut last: Option<Vec<String>> = None;
    let mut save_error: Option<String> = None;
    {
        let mut terminal = ratatui::try_init().context("cannot start the terminal screen")?;
        let _restore = Restore;
        let mut round = 0;
        loop {
            let feed = feeder.words(args, next_seed(args.seed, round, random_seed()), &phrases);
            let requeue = if let Feeder::Adaptive(a) = &feeder {
                Some((a.cfg.respace_gap, a.fillers.clone()))
            } else {
                None
            };
            let mut session = make_session(feed.words, args, requeue, &codes);
            ctx.hooks.run(HookEvent::BeforeTest);
            let clock = Instant::now();
            let outcome = type_loop(&mut terminal, &mut session, &clock, args.quiet, limit)?;
            ctx.hooks.run(HookEvent::AfterTest);
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let Some(record) = conclude(&session, outcome, source.name(), now) else {
                if outcome == Outcome::Restart {
                    round += 1;
                    continue;
                }
                break;
            };
            if !args.no_save
                && let Err(e) = history::append(&paths::history_file(), &record)
            {
                save_error = Some(format!("could not save the run to {}: {e:#}", paths::history_file().display()));
            }
            let results = session.results(session.finished_at.unwrap_or(0.0));
            let hints = if ctx.hints(args) {
                hints_for(&results, &ctx.dicts)
            } else {
                HashMap::new()
            };
            let mut extra = report::Extra {
                stumbled: session.unclean(),
                ..report::Extra::default()
            };
            let mut vocab_line = String::new();
            if let (Feeder::Adaptive(a), Outcome::Finished, false) = (&mut feeder, outcome, args.no_adapt) {
                let (summary, before, saved) = a.learn(&session, feed.tracked);
                vocab_line = summary.line();
                extra = report::compose(&before, &a.vocab, session.unclean(), a.cfg.mastery_accuracy);
                if let Err(e) = saved {
                    save_error = Some(format!("could not save {}: {e:#}", vocab::file().display()));
                }
            }
            let progress = if let Feeder::Adaptive(a) = &feeder {
                tiers::progress(&a.tiers, &a.vocab)
            } else {
                None
            };
            let wordset = progress.as_ref().map_or_else(|| source.name().to_owned(), |p| p.label.clone());
            let test = format!(
                "{}  {wordset}",
                args.time
                    .map_or_else(|| format!("{} words", session.words.len()), |t| format!("{t}s"))
            );
            let mut screen = report::Screen::new(
                &results,
                test,
                session.chars(),
                session.series(session.finished_at.unwrap_or(0.0)),
                extra,
                progress,
                hints,
            );
            screen.vocab_line = vocab_line;
            let lines = report::text_lines(&screen);
            if outcome == Outcome::Quit {
                // Esc or Ctrl-C quit at once, with no results screen
                last = Some(lines);
                break;
            }
            let after = show_results(&mut terminal, &screen)?;
            last = Some(lines);
            if after == After::Quit {
                break;
            }
            round += 1;
        }
    }
    match last {
        Some(lines) => {
            for line in &lines {
                println!("{line}");
            }
        }
        None => println!("no words typed"),
    }
    for w in ctx.hooks.take_warnings() {
        eprintln!("{w}");
    }
    if let Some(e) = save_error {
        eprintln!("{e}");
    }
    Ok(())
}

/// The typing session of a feed. The feed is fixed when the test starts; only with `--requeue`
/// (and an adaptive feed, whose base words are the `requeue` pair: gap and replaceable words) does
/// a stumbled or wrong word come back later in place of a base word. In accept mode `codes` (word
/// to its codes) are right answers too.
fn make_session(
    words: Vec<String>,
    args: &Args,
    requeue: Option<(usize, HashSet<String>)>,
    codes: &HashMap<String, Vec<String>>,
) -> Session {
    let wanted: HashMap<String, Vec<String>> = words
        .iter()
        .filter_map(|w| codes.get(&w.to_lowercase()).map(|c| (w.to_lowercase(), c.clone())))
        .collect();
    let session = Session::new(words, args.stop_on_error);
    let session = if wanted.is_empty() { session } else { session.with_codes(wanted) };
    match requeue {
        Some((gap, fillers)) if args.requeue => session.with_requeue(gap, fillers),
        _ => session,
    }
}

/// How a test ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    /// The feed or the clock ran out
    Finished,
    /// Esc or Ctrl-C: the results so far
    Quit,
    /// Tab: abandoned for a fresh feed
    Restart,
}

/// The history record of a test, if it leaves one: an abandoned (restarted) test, or one with no
/// words typed, does not.
fn conclude(session: &Session, outcome: Outcome, source: &str, timestamp: u64) -> Option<history::Record> {
    if outcome == Outcome::Restart {
        return None;
    }
    let results = session.results(session.finished_at.unwrap_or(0.0));
    (results.words > 0).then(|| history::Record::from_results(&results, source, timestamp, outcome == Outcome::Finished))
}

/// The seed of test number `round` (0 is the first): `--seed` plus the round, so a seeded session
/// repeats exactly; without `--seed`, `random`.
const fn next_seed(seed: Option<u64>, round: u64, random: u64) -> u64 {
    match seed {
        Some(s) => s.wrapping_add(round),
        None => random,
    }
}

fn random_seed() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64)
}

/// A test's words and how many of its first words the vocabulary tracks (the planned ones: the
/// phrase words of `--phrases` come after them and are not tracked).
struct Feed {
    words: Vec<String>,
    tracked: usize,
}

/// Where a test's words come from.
enum Feeder {
    /// Bands of units drawn at random
    Fixed(Vec<Vec<String>>),
    /// The adaptive vocabulary
    Adaptive(Box<Adaptive>),
}

/// Share of an adaptive test given to the dictionaries' phrases with `--phrases`.
const PHRASE_SHARE: usize = 5;
/// Fewest words in a test for phrases to be mixed into it.
const PHRASE_MIN_TEST: usize = 10;

impl Feeder {
    fn words(&mut self, args: &Args, seed: u64, phrases: &[String]) -> Feed {
        match self {
            Self::Fixed(pool) => {
                let words = feed::build(pool.clone(), &feed_opts(args, seed));
                Feed {
                    tracked: words.len(),
                    words,
                }
            }
            Self::Adaptive(a) => {
                let size = feed_opts(args, seed).count;
                let extra = if phrases.is_empty() || size < PHRASE_MIN_TEST {
                    0
                } else {
                    size / PHRASE_SHARE
                };
                let mut words = a.feed(args, seed, size - extra);
                let tracked = words.len();
                if extra > 0 {
                    // a phrase is never cut short: only those that fit the room are drawn
                    let fitting: Vec<String> = phrases.iter().filter(|p| p.split_whitespace().count() <= extra).cloned().collect();
                    if !fitting.is_empty() {
                        let opts = feed::Opts {
                            count: extra,
                            no_repeat: false,
                            weighted: false,
                            ..feed_opts(args, seed)
                        };
                        words.extend(feed::build(vec![fitting], &opts));
                    }
                }
                Feed { words, tracked }
            }
        }
    }
}

/// The adaptive source: the word pool, the vocabulary and what teaches it.
struct Adaptive {
    pool: pool::Pool,
    vocab: Vocab,
    cfg: adaptive::Config,
    /// Whether finished tests are written to vocab.json
    save: bool,
    /// The easy-base words of the current feed: a missed word may replace one of them
    fillers: HashSet<String>,
    /// The pool cut into levels, for the results screen's bars
    tiers: Vec<tiers::Tier>,
}

impl Adaptive {
    fn open(args: &Args, ctx: &Ctx) -> Result<Self> {
        let (lists, warnings) = crate::config::load_lists(&ctx.cfg.lists);
        for w in warnings {
            eprintln!("{w}");
        }
        let opts = pool::Opts {
            names: args.names,
            deep: args.deep,
            lists,
            user_words: ctx.dicts.words(),
        };
        let pool = pool::Pool::open(&paths::data_dir(), &opts);
        for name in pool.missing.iter().filter(|n| args.deep || *n != "english_450k") {
            eprintln!("note: Monkeytype list {name} is not downloaded (run `typeladder fetch`); the pool is without it");
        }
        let path = vocab::file();
        let mut vocab = Vocab::load(&path)?;
        let base = adaptive::Config {
            new_per_session: args.new_per_session,
            learning_cap: args.learning_cap,
            ..adaptive::Config::default()
        };
        let cfg = vocab::with_gate(&base, &vocab, args.gate);
        if let Some(removed) = vocab::migrate(&mut vocab, &pool.filter) {
            if !args.no_adapt {
                if path.exists() {
                    let bak = path.with_extension("json.bak");
                    std::fs::copy(&path, &bak).with_context(|| format!("cannot back up {} to {}", path.display(), bak.display()))?;
                }
                vocab.save(&path)?;
            }
            eprintln!(
                "\x1b[2mvocab cleaned: {} words hidden (typeladder vocab clean --dry-run to review)\x1b[0m",
                removed.len()
            );
        }
        if !vocab.seeded {
            let usage = ctx.cfg.seed.usage.as_deref().map(|u| paths::expand_home(std::path::Path::new(u)));
            let added = vocab::ensure_seeded(&mut vocab, usage.as_deref(), &cfg, today(), &pool.filter);
            if added > 0 {
                eprintln!("first run: {added} words read from your history into {}", path.display());
            }
            if !args.no_adapt {
                vocab.save(&path)?;
            }
        }
        let tiers = pool.tiers(args.deep);
        Ok(Self {
            pool,
            vocab,
            cfg,
            save: !args.no_adapt,
            fillers: HashSet::new(),
            tiers,
        })
    }

    fn feed(&mut self, args: &Args, seed: u64, size: usize) -> Vec<String> {
        let (min, max, lower) = (args.min_len, args.max_len, args.lowercase_only);
        let (filter, hidden) = (self.pool.filter.clone(), self.vocab.hidden.clone());
        let keep = move |w: &str| {
            let n = w.chars().count();
            n >= min && n <= max && !(lower && w.chars().any(char::is_uppercase)) && filter.accepts(w) && !hidden.contains(w)
        };
        let plan = adaptive::plan_feed(&self.vocab, &mut self.pool, &self.cfg, today(), size, seed, &keep);
        self.fillers = plan.fillers.into_iter().collect();
        plan.words
    }

    /// Teaches the vocabulary a finished test and saves it. Returns what changed, the vocabulary
    /// as it was before, and whether it was saved. Only the first `tracked` words of the test
    /// are the vocabulary's.
    fn learn(&mut self, session: &Session, tracked: usize) -> (adaptive::Summary, Vocab, Result<()>) {
        let before = self.vocab.clone();
        let summary = adaptive::apply(&mut self.vocab, &attempts_of(session, tracked), &self.cfg, today());
        let saved = if self.save { self.vocab.save(&vocab::file()) } else { Ok(()) };
        (summary, before, saved)
    }
}

/// The attempts of a test, in order: the words `--stop-on-error` refused (misses), then each
/// committed word with the time since the one before it (none for the first, which carries no
/// recall time). An attempt is a hit only when the word was right and needed no correction. Only
/// the first `tracked` words of the test count (the rest are phrases).
fn attempts_of(session: &Session, tracked: usize) -> Vec<adaptive::Attempt> {
    let planned: HashSet<&str> = session.words.iter().take(tracked).map(String::as_str).collect();
    let mut out: Vec<adaptive::Attempt> = session
        .refusals()
        .iter()
        .filter(|m| planned.contains(m.word.as_str()))
        .map(|m| adaptive::Attempt {
            word: m.word.clone(),
            hit: false,
            stumbles: 0,
            secs: None,
        })
        .collect();
    out.extend(session.done.iter().enumerate().take(tracked).map(|(i, d)| adaptive::Attempt {
        word: d.target.clone(),
        hit: d.clean(),
        stumbles: d.stumbles,
        secs: (i > 0).then_some(d.secs),
    }));
    out
}

fn today() -> u32 {
    vocab::day_of(
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        vocab::local_offset(),
    )
}

/// `N stuck words: typeladder vocab stuck`, when there are any.
fn stuck_line(n: usize) -> Option<String> {
    (n > 0).then(|| format!("{n} stuck word{}: typeladder vocab stuck", if n == 1 { "" } else { "s" }))
}

/// Missed words of the recorded runs, one entry per miss: all `vocab seeding` knows of them.
pub fn history_misses(path: &std::path::Path) -> Vec<String> {
    history::read(path).into_iter().flat_map(|r| r.missed).collect()
}

/// Restores the terminal when dropped, however the run ends. (`ratatui::try_init` also installs a
/// panic hook that does the same.)
struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

fn feed_opts(args: &Args, seed: u64) -> feed::Opts {
    let size = args.time.map_or(args.count, |t| {
        usize::try_from(t * TIMED_WORDS_PER_SECOND).unwrap_or(usize::MAX).max(200)
    });
    feed::Opts {
        count: size,
        seed,
        no_repeat: args.no_repeat,
        lowercase_only: args.lowercase_only,
        min_len: args.min_len,
        max_len: args.max_len,
        weighted: args.weighted,
    }
}

/// The source's words, read once: the bands each test's feed is drawn from. `phrases` (the
/// dictionaries' multiword entries) join a fixed source's pool when asked for.
fn make_pool(args: &Args, ctx: &Ctx, source: Source, phrases: &[String]) -> Result<Vec<Vec<String>>> {
    let mut bands = match source {
        Source::Adaptive => bail!("the adaptive source has no fixed pool"),
        Source::Common => {
            let spec = args.bands.as_deref().or(args.rank.as_deref()).unwrap_or("1-1000");
            let bands = feed::parse_bands(spec)?;
            let (lists, _) = crate::config::load_lists(&ctx.cfg.lists);
            let opts = pool::Opts {
                names: args.names,
                deep: args.deep,
                lists,
                user_words: ctx.dicts.words(),
            };
            let mut pool = pool::Pool::open(&paths::data_dir(), &opts);
            let ranked = sources::common(&mut pool, feed::reach(&bands))?;
            feed::band_units(&ranked, &bands)
        }
        Source::File => {
            let Some(path) = &args.file else {
                bail!("--source file needs --file PATH")
            };
            vec![sources::file(path, args.names)?]
        }
        Source::Phrases => vec![sources::phrases(ctx.dicts.phrases())?],
    };
    if source != Source::Phrases && !phrases.is_empty() {
        bands.push(sources::phrases(phrases.to_vec())?);
    }
    Ok(bands)
}

/// How often the screen is redrawn with no key pressed, so the clock and speed keep moving.
const REDRAW: Duration = Duration::from_millis(250);

/// Runs the screen until the feed or the clock ends, the typist quits or restarts.
fn type_loop(terminal: &mut DefaultTerminal, session: &mut Session, clock: &Instant, quiet: bool, limit: Option<f64>) -> Result<Outcome> {
    loop {
        let now = clock.elapsed().as_secs_f64();
        if let (Some(limit), Some(start)) = (limit, session.started)
            && now - start >= limit
        {
            session.finish(start + limit);
        }
        terminal.draw(|f| {
            ui::draw(
                f,
                &ui::View {
                    session,
                    now,
                    quiet,
                    limit,
                },
            );
        })?;
        if session.finished() {
            return Ok(Outcome::Finished);
        }
        if !event::poll(REDRAW)? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match translate(key) {
                Key::Quit => {
                    session.finish(clock.elapsed().as_secs_f64());
                    return Ok(Outcome::Quit);
                }
                Key::Restart => return Ok(Outcome::Restart),
                Key::Input(input) => session.apply(input, clock.elapsed().as_secs_f64()),
                Key::Ignore => {}
            },
            _ => {}
        }
    }
}

enum Key {
    Quit,
    Restart,
    Input(Input),
    Ignore,
}

/// A key press during a test as the engine sees it. Ctrl-Backspace arrives as Ctrl-H (or
/// Backspace with a modifier) and Ctrl-W deletes a word; Ctrl-C and Esc quit, Tab restarts, and
/// `q` is an ordinary letter.
fn translate(key: KeyEvent) -> Key {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let word = key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    match key.code {
        KeyCode::Esc => Key::Quit,
        KeyCode::Tab => Key::Restart,
        KeyCode::Char('c') if ctrl => Key::Quit,
        KeyCode::Char('w' | 'h') if ctrl => Key::Input(Input::DeleteWord),
        KeyCode::Backspace if word => Key::Input(Input::DeleteWord),
        KeyCode::Backspace => Key::Input(Input::Backspace),
        KeyCode::Enter => Key::Input(Input::Commit),
        KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => Key::Input(Input::Char(c)),
        _ => Key::Ignore,
    }
}

/// What a key on the results screen asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum After {
    Quit,
    Restart,
}

/// Seconds after the results appear (and after each key swallowed) in which keys still arriving
/// from the run cannot dismiss the screen.
const RESULTS_QUIET: f64 = 0.7;

/// The results screen's key handling, with the time passed in. Ctrl-C and Esc quit at once (a
/// steno burst never sends them); inside the quiet window every other key is swallowed (and
/// extends it); after it, `q` quits and Tab and Enter restart.
struct Gate {
    quiet_until: f64,
}

impl Gate {
    const fn new(now: f64) -> Self {
        Self {
            quiet_until: now + RESULTS_QUIET,
        }
    }

    fn key(&mut self, key: KeyEvent, now: f64) -> Option<After> {
        if key.code == KeyCode::Esc || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)) {
            return Some(After::Quit);
        }
        if now < self.quiet_until {
            self.quiet_until = now + RESULTS_QUIET;
            return None;
        }
        if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return None;
        }
        match key.code {
            KeyCode::Char('q') => Some(After::Quit),
            KeyCode::Tab | KeyCode::Enter => Some(After::Restart),
            _ => None,
        }
    }
}

/// Shows the results until the typist quits or asks for another test.
fn show_results(terminal: &mut DefaultTerminal, screen: &report::Screen) -> Result<After> {
    terminal.draw(|f| screen::draw(f, screen))?;
    let clock = Instant::now();
    let mut gate = Gate::new(0.0);
    loop {
        if !event::poll(REDRAW)? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                if let Some(after) = gate.key(k, clock.elapsed().as_secs_f64()) {
                    return Ok(after);
                }
            }
            Event::Resize(..) => {
                terminal.draw(|f| screen::draw(f, screen))?;
            }
            _ => {}
        }
    }
}

/// The hint of each missed, slow or stumbled word the dictionaries know.
fn hints_for(results: &engine::Results, dicts: &Dictionaries) -> HashMap<String, String> {
    let wanted: HashSet<&str> = results
        .slow
        .iter()
        .map(|(w, _)| w.as_str())
        .chain(results.misses.iter().map(|m| m.word.as_str()))
        .chain(results.stumbled.iter().map(|(w, _)| w.as_str()))
        .collect();
    dicts.hints(&wanted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::Done;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn keys_map_to_inputs() {
        let none = KeyModifiers::NONE;
        assert!(matches!(translate(key(KeyCode::Char('a'), none)), Key::Input(Input::Char('a'))));
        assert!(matches!(translate(key(KeyCode::Char(' '), none)), Key::Input(Input::Char(' '))));
        assert!(matches!(translate(key(KeyCode::Enter, none)), Key::Input(Input::Commit)));
        assert!(matches!(translate(key(KeyCode::Backspace, none)), Key::Input(Input::Backspace)));
        assert!(matches!(
            translate(key(KeyCode::Backspace, KeyModifiers::CONTROL)),
            Key::Input(Input::DeleteWord)
        ));
        assert!(matches!(
            translate(key(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            Key::Input(Input::DeleteWord)
        ));
        assert!(matches!(
            translate(key(KeyCode::Char('h'), KeyModifiers::CONTROL)),
            Key::Input(Input::DeleteWord)
        ));
        assert!(matches!(translate(key(KeyCode::Char('c'), KeyModifiers::CONTROL)), Key::Quit));
        assert!(matches!(translate(key(KeyCode::Esc, none)), Key::Quit));
        assert!(matches!(translate(key(KeyCode::Tab, none)), Key::Restart));
        // `q` is an ordinary letter during a test
        assert!(matches!(translate(key(KeyCode::Char('q'), none)), Key::Input(Input::Char('q'))));
        assert!(matches!(translate(key(KeyCode::Char('x'), KeyModifiers::CONTROL)), Key::Ignore));
    }

    #[test]
    fn the_results_screen_keys() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let after = |gate: &mut Gate, code, mods, now| gate.key(key(code, mods), now);
        let mut g = Gate::new(10.0);
        // inside the 0.7 s window: stray keys do nothing, and extend it
        assert_eq!(after(&mut g, KeyCode::Char('q'), none, 10.3), None);
        assert_eq!(
            after(&mut g, KeyCode::Tab, none, 10.8),
            None,
            "the window was extended by the key at 10.3 and 10.8"
        );
        assert_eq!(after(&mut g, KeyCode::Enter, none, 10.9), None);
        // Esc and Ctrl-C work inside the window
        assert_eq!(after(&mut g, KeyCode::Esc, none, 10.95), Some(After::Quit));
        assert_eq!(after(&mut g, KeyCode::Char('c'), ctrl, 10.95), Some(After::Quit));
        // just after it, everything works at once
        assert_eq!(after(&mut g, KeyCode::Char('q'), none, 11.7), Some(After::Quit));
        let mut g = Gate::new(0.0);
        assert_eq!(after(&mut g, KeyCode::Esc, none, 0.1), Some(After::Quit));
        assert_eq!(after(&mut g, KeyCode::Tab, none, 0.7), Some(After::Restart));
        assert_eq!(after(&mut g, KeyCode::Enter, none, 5.0), Some(After::Restart));
        assert_eq!(after(&mut g, KeyCode::Char('c'), ctrl, 5.0), Some(After::Quit));
        assert_eq!(after(&mut g, KeyCode::Char('x'), none, 5.0), None);
        assert_eq!(after(&mut g, KeyCode::Char(' '), none, 5.0), None);
        assert_eq!(after(&mut g, KeyCode::Char('q'), ctrl, 5.0), None);
    }

    #[test]
    fn restart_advances_a_given_seed_and_randomises_otherwise() {
        assert_eq!(next_seed(Some(7), 0, 999), 7);
        assert_eq!(next_seed(Some(7), 1, 999), 8);
        assert_eq!(next_seed(Some(7), 2, 999), 9);
        assert_eq!(next_seed(Some(u64::MAX), 1, 999), 0, "wraps");
        assert_eq!(next_seed(None, 3, 999), 999);
        // different seeds give different feeds, the same seed the same feed
        let pool = vec![(0..500).map(|i| format!("w{i}")).collect::<Vec<_>>()];
        let opts = |seed| feed::Opts {
            count: 25,
            seed,
            no_repeat: false,
            lowercase_only: false,
            min_len: 1,
            max_len: 30,
            weighted: false,
        };
        let a = feed::build(pool.clone(), &opts(7));
        assert_eq!(a, feed::build(pool.clone(), &opts(7)));
        assert_ne!(a, feed::build(pool, &opts(8)));
        assert_eq!(a.len(), 25);
    }

    #[test]
    fn a_restart_resets_the_session_and_leaves_no_history() {
        let words: Vec<String> = ["the", "cat", "sat"].map(String::from).to_vec();
        let mut s = Session::new(words.clone(), false);
        for c in "the ca".chars() {
            s.apply(Input::Char(c), 2.0);
        }
        assert!(s.started.is_some() && s.current() == 1 && !s.typed.is_empty());
        // abandoned by Tab: no record
        assert!(
            conclude(&s, Outcome::Restart, "common", 1).is_none(),
            "an abandoned test is not saved"
        );
        // the next test is a fresh session: nothing carried over, the clock not started
        let fresh = Session::new(words, false);
        assert!(fresh.started.is_none() && fresh.current() == 0 && fresh.typed.is_empty() && !fresh.finished());
        assert_eq!(fresh.results(5.0).words, 0);
        // Esc with words typed does leave a record, marked incomplete; with none it does not
        s.finish(4.0);
        let rec = conclude(&s, Outcome::Quit, "common", 1).expect("words typed");
        assert!(!rec.complete && rec.count == 1);
        assert!(conclude(&fresh, Outcome::Quit, "common", 1).is_none(), "no words typed, no record");
        let mut done = Session::new(vec!["a".into()], false);
        done.apply(Input::Char('a'), 1.0);
        assert!(conclude(&done, Outcome::Finished, "common", 1).expect("finished").complete);
    }

    #[test]
    fn a_session_becomes_attempts_with_times_except_the_first_word() {
        let words: Vec<String> = ["the", "cat", "sat"].map(String::from).to_vec();
        let mut s = Session::new(words, false);
        for (c, t) in "the "
            .chars()
            .map(|c| (c, 1.0))
            .chain("cax ".chars().map(|c| (c, 2.5)))
            .chain("sat".chars().map(|c| (c, 3.0)))
        {
            s.apply(Input::Char(c), t);
        }
        let got = attempts_of(&s, usize::MAX);
        assert_eq!(
            got,
            vec![
                adaptive::Attempt {
                    word: "the".into(),
                    hit: true,
                    stumbles: 0,
                    secs: None
                },
                adaptive::Attempt {
                    word: "cat".into(),
                    hit: false,
                    stumbles: 0,
                    secs: Some(1.5)
                },
                adaptive::Attempt {
                    word: "sat".into(),
                    hit: true,
                    stumbles: 0,
                    secs: Some(0.5)
                },
            ]
        );
        // stop on error: the refused commit is a miss, then the right word is a hit
        let mut s = Session::new(vec!["cat".into()], true);
        for c in "cax ".chars() {
            s.apply(Input::Char(c), 1.0);
        }
        for _ in 0..3 {
            s.apply(Input::Backspace, 1.0);
        }
        for c in "cat".chars() {
            s.apply(Input::Char(c), 2.0);
        }
        let got = attempts_of(&s, usize::MAX);
        assert_eq!(
            (got[0].hit, got[1].hit, got.len()),
            (false, true, 2),
            "the refusal is the miss; deleting the refused word is not a second one: {got:?}"
        );
    }

    #[test]
    fn the_feed_is_fixed_unless_requeue_is_given() {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: Args,
        }
        let fillers = || Some((3, ["dd", "ee"].iter().map(|w| (*w).to_owned()).collect::<HashSet<String>>()));
        let words: Vec<String> = ["aa", "bb", "cc", "dd", "ee"].map(String::from).to_vec();
        let miss = |s: &mut Session| {
            for c in "xx ".chars() {
                s.apply(Input::Char(c), 1.0);
            }
        };
        let default = Cli::parse_from(["x"]).args;
        assert!(!default.requeue, "off by default");
        let mut fixed = make_session(words.clone(), &default, fillers(), &HashMap::new());
        miss(&mut fixed);
        assert_eq!(fixed.words, words, "a missed word replaces nothing: {:?}", fixed.words);
        let on = Cli::parse_from(["x", "--requeue"]).args;
        let mut requeued = make_session(words.clone(), &on, fillers(), &HashMap::new());
        miss(&mut requeued);
        assert_eq!(
            requeued.words,
            ["aa", "bb", "cc", "aa", "ee"].map(String::from),
            "--requeue replaces a later base word: {:?}",
            requeued.words
        );
        // a source without base words has nothing to replace, flag or not
        let mut plain = make_session(words.clone(), &on, None, &HashMap::new());
        miss(&mut plain);
        assert_eq!(plain.words, words);
    }

    #[test]
    fn a_finished_session_becomes_the_results_screen_numbers() {
        let words: Vec<String> = ["the", "cat", "sat", "mat"].map(String::from).to_vec();
        let mut s = Session::new(words, false);
        for (c, t) in "the "
            .chars()
            .map(|c| (c, 1.0))
            .chain("cax ".chars().map(|c| (c, 2.2)))
            .chain("sat ".chars().map(|c| (c, 3.4)))
            .chain("mat".chars().map(|c| (c, 4.0)))
        {
            s.apply(Input::Char(c), t);
        }
        assert!(s.finished());
        let r = s.results(s.finished_at.unwrap());
        let screen = report::Screen::new(
            &r,
            "4 words  common".into(),
            s.chars(),
            s.series(s.finished_at.unwrap()),
            report::Extra {
                stumbled: s.unclean(),
                ..report::Extra::default()
            },
            None,
            HashMap::new(),
        );
        assert_eq!(
            screen.chars,
            engine::Chars {
                correct: 3 + 1 + 3 + 1 + 3,
                incorrect: 1,
                extra: 0,
                missed: 0
            },
            "the, sat, mat right with their spaces; cax has one wrong letter"
        );
        assert_eq!(screen.series.raw.len(), 3, "first key at 1.0, last at 4.0");
        assert_eq!(screen.extra.stumbled, [("cat".to_owned(), 1)]);
        let text = report::text_lines(&screen).join("\n");
        assert!(
            text.contains("characters 11/1/0/0") && text.contains("stumbled  cat") && text.contains("test 4 words  common"),
            "{text}"
        );
    }

    #[test]
    fn count_defaults_to_25() {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: Args,
        }
        assert_eq!(Cli::parse_from(["x"]).args.count, 25);
    }

    #[test]
    fn the_history_ends_with_the_stuck_word_count() {
        assert_eq!(stuck_line(0), None);
        assert_eq!(stuck_line(1).as_deref(), Some("1 stuck word: typeladder vocab stuck"));
        assert_eq!(stuck_line(4).as_deref(), Some("4 stuck words: typeladder vocab stuck"));
    }

    fn test_args() -> Args {
        Args {
            source: None,
            new_per_session: 3,
            learning_cap: 10,
            gate: None,
            deep: false,
            no_adapt: true,
            file: None,
            bands: None,
            rank: None,
            count: 5,
            time: None,
            seed: Some(1),
            weighted: false,
            no_repeat: false,
            lowercase_only: false,
            min_len: 1,
            max_len: 30,
            names: false,
            stop_on_error: false,
            requeue: false,
            quiet: false,
            hints: false,
            phrases: false,
            accept: None,
            no_hooks: true,
            no_fetch: true,
            no_save: true,
            history: false,
        }
    }

    #[test]
    fn a_pipe_is_refused_with_a_message() {
        // under `cargo test` stdin is not a terminal
        let args = test_args();
        if !std::io::stdin().is_terminal() {
            let ctx = Ctx {
                cfg: Config::default(),
                dicts: Dictionaries::default(),
                hooks: Hooks::off(),
            };
            let err = run(&args, &ctx).unwrap_err().to_string();
            assert!(err.contains("needs a terminal"), "{err}");
        }
    }

    fn results_with(slow: &[&str], misses: &[&str]) -> engine::Results {
        engine::Results {
            secs: 1.0,
            wpm: 0.0,
            raw: 0.0,
            accuracy: 100.0,
            word_accuracy: 100.0,
            errors: 0,
            words: 1,
            consistency: None,
            slow: slow.iter().map(|w| ((*w).to_owned(), 1.0)).collect(),
            misses: misses
                .iter()
                .map(|w| engine::Miss {
                    word: (*w).to_owned(),
                    typed: "x".into(),
                })
                .collect(),
            stumbled: vec![],
            coded: vec![],
        }
    }

    fn dictionaries(files: &[(&str, &str)]) -> Dictionaries {
        let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.join(format!("typeladder-practice-dict-{}-{}", std::process::id(), files[0].0));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let d = Dictionaries::load(&dir, &std::collections::BTreeMap::new());
        let _ = std::fs::remove_dir_all(&dir);
        d
    }

    #[test]
    fn hints_are_the_codes_of_the_slow_and_missed_words_only() {
        let d = dictionaries(&[("hintsa.codes.json", r#"{"dog": ["TKOG/*", "TKOG"], "cat": "KAT"}"#)]);
        let got = hints_for(&results_with(&["dog"], &[]), &d);
        assert_eq!(got.get("dog").map(String::as_str), Some("TKOG TKOG/*"));
        assert!(!got.contains_key("cat"), "only asked-for words: {got:?}");
        assert!(hints_for(&results_with(&["dog"], &[]), &Dictionaries::default()).is_empty());
        assert_eq!(
            hints_for(&results_with(&[], &["cat"]), &d).get("cat").map(String::as_str),
            Some("KAT")
        );
    }

    #[test]
    fn accept_mode_makes_a_code_a_right_answer_and_records_it() {
        let d = dictionaries(&[("acceptb.texts.json", r#"{"crt": "create", "dg": "dog"}"#)]);
        let codes = d.accepted_codes();
        let words: Vec<String> = ["create", "the", "dog"].map(String::from).to_vec();
        let mut s = make_session(words.clone(), &test_args(), None, &codes);
        for (c, t) in "crt "
            .chars()
            .map(|c| (c, 1.0))
            .chain("the ".chars().map(|c| (c, 2.0)))
            .chain("dog".chars().map(|c| (c, 3.0)))
        {
            s.apply(Input::Char(c), t);
        }
        assert!(s.finished());
        let r = s.results(3.0);
        assert_eq!((r.errors, r.accuracy), (0, 100.0), "{r:?}");
        assert_eq!(r.coded, [("create".to_owned(), "crt".to_owned())]);
        assert!(s.done.iter().all(Done::correct) && s.done[0].code && !s.done[1].code);
        // without accept mode the code is a miss
        let mut plain = make_session(words, &test_args(), None, &HashMap::new());
        for c in "crt ".chars() {
            plain.apply(Input::Char(c), 1.0);
        }
        assert_eq!(plain.results(1.0).errors, 1);
    }

    #[test]
    fn a_wrong_code_is_still_wrong_and_the_last_word_ends_on_a_code() {
        let d = dictionaries(&[("acceptc.texts.json", r#"{"crt": "create"}"#)]);
        let mut s = make_session(vec!["create".into()], &test_args(), None, &d.accepted_codes());
        for c in "crx ".chars() {
            s.apply(Input::Char(c), 1.0);
        }
        assert_eq!(s.results(1.0).errors, 1);
        let mut s = make_session(vec!["the".into(), "create".into()], &test_args(), None, &d.accepted_codes());
        for c in "the crt".chars() {
            s.apply(Input::Char(c), 1.0);
        }
        assert!(s.finished(), "the last word is finished the moment its code is typed");
    }

    #[test]
    fn phrase_words_after_the_planned_ones_are_not_tracked() {
        let words: Vec<String> = ["the", "cat", "of", "the"].map(String::from).to_vec();
        let mut s = Session::new(words, false);
        for c in "the cat of the".chars() {
            s.apply(Input::Char(c), 1.0);
        }
        let got = attempts_of(&s, 2);
        assert_eq!(got.iter().map(|a| a.word.as_str()).collect::<Vec<_>>(), ["the", "cat"]);
    }
}
