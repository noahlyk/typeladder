# Configuration reference

`~/.config/typeladder/config.toml` (`$XDG_CONFIG_HOME/typeladder/`, or `TYPELADDER_CONFIG_DIR`).
Optional: no file is the defaults. An unknown key is an error that names it. `typeladder where`
prints every folder.

## Folders

| | default | override |
|---|---|---|
| config | `~/.config/typeladder` | `TYPELADDER_CONFIG_DIR`, else `XDG_CONFIG_HOME` |
| dictionaries | `<config>/dictionaries/` | (follows config) |
| data (word lists) | `~/.local/share/typeladder` | `TYPELADDER_DATA_DIR`, else `XDG_DATA_HOME` |
| state | `~/.local/state/typeladder` | `TYPELADDER_STATE_DIR`, else `XDG_STATE_HOME` |

From a checkout (the current folder's `Cargo.toml` names `typeladder`) the folders are `./config`,
`./data` and `./state`, unless an env var says otherwise. State holds `vocab.json` (what the test
knows about each word) and `history.jsonl` (one line per finished test).

## Keys

### `accept`

`accept = ["text"]` (default) or `["text", "code"]`. With `"code"`, a word typed as one of its codes
from any dictionary counts as right, so you can practise a shorthand with the expander off. Only
single-word entries whose code has no spaces count. The form typed is kept: `coded` words in
`history.jsonl`. Flag: `--accept text,code`. `"text"` must stay in the list.

### `hints`

`hints = false` (default). `true` shows each stumbled, missed or slow word's codes next to it on the
results screen (`stumbled  the (-T)`; with several dictionaries `(lyk: -T, esp: tt)`). Never during
the test. Flag: `--hints`. `typeladder lookup WORD` always works.

### `phrases`

`phrases = false` (default). `true` includes the dictionaries' multiword entries as practice items:
in the adaptive source about a fifth of a test (when it has 10 words or more) is phrases, not
tracked in `vocab.json`; in `--source common` or `file` they join the pool. `--source phrases` is
phrases only. Flag: `--phrases`.

### `auto_fetch`

`auto_fetch = true` (default). Download Monkeytype's lists (`english_1k` to `english_25k`, about
600 KB) when a test starts and they are missing, saying so. A failed download falls back to what
is on disk (the 200-word list is built in), and is not tried again for a day. `--deep` also
fetches the 8 MB `english_450k`. `false`, or `--no-fetch`: never; run `typeladder fetch`.

### `lists`

`lists = ["~/words.txt", "~/more.json"]`. Your own word lists, in order, each a tier after
Monkeytype's 25k (tier key `list:NAME` after the file name, goal `typeladder vocab goal list:words`).
A `.json` file is Monkeytype's format (`{"words": [...]}` or a bare array); anything else is words
separated by spaces or lines, `#` starts a comment. The words must be plain words (letters, a few
contractions); they are practice words even if Monkeytype lacks them.

### `[seed]`

`usage = "~/usage.txt"`. Read once when `vocab.json` is first made (and by `typeladder vocab seed`):
one `word attempts errors [days]` per line, `#` comments. Words with 20+ attempts and few errors start
in review, the clumsiest start in learning. Without it the vocabulary starts empty at the top of the
200 list, plus the missed words of an existing `history.jsonl`.

### `[hooks]`

```toml
[hooks]
before_session = "espanso stop"
after_session  = "espanso start"
before_test    = ""
after_test     = ""
```

Shell commands (`sh -c`, no input, output discarded), 3 seconds each; a failure or a timeout is a
one-line warning, never fatal. `before_session` runs once before the first test, `after_session`
once after the last: on quit, Ctrl-C or SIGTERM, an error and a panic too (best effort, not for
SIGKILL). `before_test` / `after_test` run around every test, a restart (`tab`) included.
`--no-hooks` skips all of them. They run only when the test is interactive (a terminal).

espanso: `before_session = "espanso stop"`, `after_session = "espanso start"`.
keymux: none needed (it types ordinary characters). It has no command-line switch for leaving the steno
layer (that is the `{:engine:suspend}` stroke), so there is no stock keymux hook; a script of your own
goes in the same way as `espanso stop`.

### `[dictionary."NAME"]`

`direction = "codes"` or `"texts"`. Overrides how the file `NAME.*` is read (the name is the file
name without `.codes`/`.texts` and the extension).

## Dictionaries

Every `.json`, `.yaml`, `.yml` and `.toml` file in the dictionaries folder is one dictionary,
named after the file. A file that cannot be read is a warning, the rest load.

| shape | example |
|---|---|
| flat map | `{"create": "KRAET"}`; a value may be a list of codes |
| espanso match file | `matches:` list of `{trigger \| triggers, replace}` |
| TOML table | `create = "KRAET"` (nested tables are flattened) |
| list of records | `[{"from": "create", "to": "KRAET"}]`, `key`/`value`, `trigger`/`replace` |

**Direction.** `codes` is text to code (`{"create": "KRAET"}`), `texts` is code to text
(`{"KRAET": "create"}`). First rule that applies: `[dictionary."NAME"]` in config; the file name
(`lyk.codes.json`, `plover.texts.json`); the format (espanso matches are always `texts`: the trigger
is the code); a guess. The guess: if 60% or more of the keys look like strokes (no spaces, capitals,
digits and `-*/#^+~` only) and fewer values do, `texts`; the reverse, `codes`; otherwise the shorter
side is the code (a trigger is shorter than what it expands to); about the same length is a tie, and
a tie is `texts`. `typeladder lookup` prints which rule decided each file.

**Phrases** are entries whose text has more than one word; any dictionary may hold them (the lyk side
writes a second file, `lyk-phrases.codes.json`).

**Practice words.** The plain single words of the dictionaries count as practice words for
`typeladder vocab clean` (they are not added to the pool; the pool is Monkeytype's lists and `lists`).

## Command line

`typeladder [practice]`: `--count N` (25), `--time SECS`, `--seed N`, `--source adaptive|common|file|phrases`,
`--file PATH`, `--rank MIN-MAX`, `--bands`, `--new-per-session N` (2), `--learning-cap N` (10),
`--gate PERCENT`, `--deep`, `--names`, `--no-adapt`, `--stop-on-error`, `--requeue`, `--quiet`,
`--min-len`, `--max-len`, `--lowercase-only`, `--weighted`, `--no-repeat`, `--no-save`, `--history`,
`--hints`, `--phrases`, `--accept`, `--no-hooks`, `--no-fetch`.
`typeladder vocab`: `--json`, `--names`, `--deep`; actions `know`, `skip`, `forget`, `reset --yes`,
`gate PERCENT|default`, `stuck`, `clean [--dry-run]`, `unhide`, `goal [TIER|N|clear]`, `spread`,
`seed [--usage FILE]`.
`typeladder import --from-lyk [--dry-run] [--force] [--lyk-state DIR]`.
