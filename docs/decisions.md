# Decisions

The log of why, newest last. Each entry is short; the code and `docs/config.md` say what.

## Extracted from lyk-steno-theory (2026-11)

`typeladder` is the adaptive test of `lyk practice` and `lyk vocab`, made independent of any engine.
The test never sees strokes: an engine types finished words as characters, so one program serves
steno, text expanders and plain keyboards. The `vocab.json` schema is unchanged, so an existing lyk
vocabulary loads as it is (`typeladder import --from-lyk` copies it).

## No hints by default

Showing the code while you type destroys recall (the user's rule, from experience). Hints exist
only on the results screen (`--hints`, `hints = true`) and in `typeladder lookup`. Never mid-test.

## Dictionaries are hints and answers, never the pool

Dictionaries (any of json, yaml, toml; espanso matches included) feed hints, phrases and accepted
codes. They are not a source of practice words: a 100,000-entry steno dictionary would bury the
ladder. Their plain single words only count as practice words for `vocab clean`, so a vocabulary
imported from lyk is not cleaned away. Direction (text to code or code to text) comes from config,
then the file name, then the format, then a guess that says it guessed.

## Accept mode instead of an engine switch

To practise a shorthand with the expander off, typing the shorthand must count. `accept = ["text",
"code"]` makes a word's dictionary codes right answers and records which form was typed. The engine
is paused by hooks, not by the program: typeladder knows no engine.

## Hooks are `sh -c` with a 3-second limit

`before_session`/`after_session`/`before_test`/`after_test` are plain shell strings so any engine
works (`espanso stop`, a keymux command). `after_session` runs once on every way out we can catch
(normal end, error, panic, Ctrl-C, SIGTERM). A failing hook is a warning; the test is never blocked.

## The pool is Monkeytype's lists, not a frequency file

lyk merged its own subtitle frequency list with Monkeytype's. Here the order is Monkeytype's
`english`, `1k`, `5k`, `10k` (each adding what earlier lists lack), then the scrambled `25k`, then
your lists, then with `--deep` the 450k dictionary. lyk's `--wordlist`, `--monkeytype-weight` and
"lyk top 20k" tier went with the frequency file. The 200-word list is built into the binary, so the
first run works offline and downloads the rest.

## Not carried over from lyk

Anything that needs lyk's dictionary, stroke or ergonomics code: outlines (`--outlines`, replaced by
dictionary hints), the steno tape seeding and `vocab stuck`'s wrong-word and cost columns, the
`mistakes` and `quotes` sources, the frequency-rank route into the practice-word filter and its
name list. First-run seeding is the Monkeytype baseline plus an optional usage file
(`word attempts errors [days]` lines); parsing keymux's tape needs lyk's replay of undos and was not
worth porting.

## Erasing a right word is a redo, not a stumble

A correction is a stumble only when the text it starts erasing is wrong (a wrong word, or part of
the word). Two things the keymux tape showed (2026-11): steno engines put the space before each
word, so `*` over a misstroke also deleted the space and reopened the word before, which was charged
a stumble it never made (now: a right word that is only reopened counts for nothing); and the typist
undoes strokes that felt wrong but were right, about a quarter of the outputs of the words that would
not master, each of which cost a whole clean attempt. Mastery measured that habit instead of recall.
Erasing the whole right word is now a redo, worth a quarter of a miss (`REDO_CREDIT` 0.75, mark `h`
in a word's window): the habit still costs something, in step with how often it happens, and mastery
takes up to four redos in ten with no miss. A redone review word holds its level.

## The word before or after, typed in place, is a slip

The aim is to score what the typist meant to type. Typing the neighbouring word (reading one ahead,
or the last word again) shows the word is known; charging the due word a miss for it measured
reading, not recall. A slip is no attempt for either word. Only the immediate neighbours count: a
misstroke that happens to write a word two places away is still a miss. The cost: a misstroke that
writes exactly the next word (`fine` for `find` before `fine`) also goes unscored, which is rare.

## Formatting and lints

rustfmt with `max_width = 140`, clippy pedantic and nursery with `-D warnings`; `too_many_lines`
is allowed (the screen, the planner and the commands read top to bottom).
