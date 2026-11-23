# typeladder

An adaptive typing test. It builds your vocabulary from the commonest word to the rarest
(Monkeytype's English 200, then 1k, 5k, 10k, 25k, then your own lists), re-drills what you miss,
retires what you have mastered and brings it back later for review. Every test ends on a
Monkeytype-style results screen: big wpm and accuracy, a per-second chart, panels for what was new,
stumbled, mastered and stuck, and level and goal bars.

It reads ordinary key presses in a terminal, so a steno engine, a text expander (espanso) and a
plain keyboard all work the same. It never knows which one you use. Your own dictionaries are
optional: they give hints, phrases and extra accepted answers.

## Install

- Arch: `makepkg -si` in a checkout (or `make pkg`), or the AUR package `typeladder` once published.
- Anywhere: `cargo build --release --locked`, or `make install` (`PREFIX=/usr/local`).
- A build from a checkout reports its commit: `typeladder 0.1.0+g0587d41`.

## 60 seconds

```
typeladder              # the test; the first run downloads Monkeytype's lists (~600 KB)
typeladder vocab        # counts, your level, the learning pool, what is due
typeladder vocab goal 5k
typeladder where        # the folders in use
```

No config, no dictionaries, no steno: that is a working setup. Keys: type, `space` commits a word,
`tab` restarts, `esc` quits, on the results screen `tab`/`enter` is another test and `q` quits.

## How it works

- **Tiers.** The pool is Monkeytype's lists in order, each adding the words earlier ones lack. A tier
  is complete at 90% of its words known (review or retired); your level is the first incomplete tier.
- **Earning words.** At most 2 new words per test, and only while fewer than 10 are being learned and
  the first attempts of your already-known words were at least 70% clean (`typeladder vocab gate N`).
- **Mastery.** A learning word becomes review at 9 clean of its last 10 attempts, and fast enough.
  A word that stays unclean is marked stuck: it leaves the learning cap and is drilled lightly.
- **Review.** Known words return after 1, 3, 7, 21, 60, 180 days; a miss moves a word down a level.
- **Stumbles count.** A word corrected before you commit it is not clean.

## Configuration

Everything is in `~/.config/typeladder/config.toml` and is optional; every key is in
[docs/config.md](docs/config.md). The three setups:

**1. Plain qwerty: nothing to do.** Run `typeladder`.

**2. espanso shorthand.** Put your match files in `~/.config/typeladder/dictionaries/` (copies or
symlinks of `~/.config/espanso/match/*.yml`). With espanso stopped during the test you type the
shorthand itself, and it counts:

```toml
accept = ["text", "code"]          # the word, or its shorthand from any dictionary
[hooks]
before_session = "espanso stop"
after_session  = "espanso start"
```

`typeladder lookup btw` shows what the dictionaries have. A shorthand that was typed is recorded in
the history as a coded word.

**3. A steno theory (lyk).** Dictionaries exported as `NAME.codes.json` (`{"word": "OUTLINE"}`) or
`NAME.texts.json` (`{"OUTLINE": "word"}`, the Plover way) go in the same folder. The steno engine
types words into the terminal, so nothing else is needed. Add `hints = true` (or `--hints`) to see
the outlines of the words you stumbled on, on the results screen only:

```toml
hints = true
```

**keymux** needs no hook: it types ordinary characters, so the test works as it is. To type raw keys
instead of strokes, leave the steno layer by hand first (the stroke bound to `{:engine:suspend}`);
keymux has no command-line switch for that, so there is no hook for it. If your setup has a
command that does what you want (a script, `systemctl --user stop ...`), put it in `[hooks]` the
same way as `espanso stop`.

No hint is ever shown during a test: seeing the code while you type destroys recall.

## Commands

| | |
|---|---|
| `typeladder` / `practice` | the test: `--count N`, `--time SECS`, `--hints`, `--phrases`, `--accept text,code`, `--deep`, `--history`, `--source adaptive\|common\|file\|phrases` |
| `vocab` | summary; `goal`, `gate`, `know`, `skip`, `forget`, `reset`, `stuck`, `clean`, `unhide`, `spread`, `seed` |
| `lookup WORD...` | each dictionary's codes for a word (or what a code is) |
| `fetch` | download Monkeytype's lists (the first run does this itself) |
| `where` | the folders in use and what decided each |
| `import --from-lyk` | copy lyk's `vocab.json` and `practice.jsonl` here (`--dry-run`, `--force`) |

## Coming from `lyk practice`

`typeladder import --from-lyk --dry-run`, then without `--dry-run`. lyk's folder is read, never
changed; an existing typeladder file is refused unless `--force` (it is kept as `.bak`). The
`vocab.json` format is the same. lyk's "top 20k" tier has no equivalent here; words from it stay in
your vocabulary and are still reviewed.

## Checks

`make check`: `cargo fmt --all --check`, `cargo clippy --release --all-targets -- -D warnings`,
`cargo test`. The tests include one that fails when `docs/config.md` forgets a config key.

GPL-3.0-or-later. See [NOTICE](NOTICE) for the Monkeytype word lists and the lyk origin.
