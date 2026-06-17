# Waymark

Waymark is a zsh-first Rust CLI for file and directory frecency tracking. It
replaces the practical parts of `fasd` used in interactive shells:

- track files and directories in SQLite
- query by ranked multi-token path matching
- import existing fasd data
- provide fasd-like zsh commands
- provide fasd-style comma completion
- keep shell hooks quiet and local to zsh option state

This repository currently contains the MVP described in
`docs/waymark-mvp.md`.

## Install

Build the binary:

```sh
cargo build --release
```

Then put `target/release/waymark` on your `PATH`.

For local development, `cargo run -- ...` works too:

```sh
cargo run -- query --kind any config
```

## Shell Setup

Load the zsh integration from your `.zshrc`:

```zsh
eval "$(waymark init zsh)"
```

The generated zsh code defines:

- `z query`: cd to the best matching directory
- `zz query`: choose a matching directory through `fzf`, with best-match fallback
- `f query`: print the best matching file
- `v query`: open the best matching file in `$EDITOR`
- `vv query`: choose a matching file through `fzf`, then open it in `$EDITOR`
- `d query`: print the best matching directory
- `a query`: print the best matching file or directory
- `s query`: list scored file and directory matches
- `sf query`: list scored file matches
- `sd query`: list scored directory matches

Interactive commands use `fzf -1 -0 --no-sort +m` so Waymark's ranked order is
preserved, single candidates are accepted immediately, empty candidate sets exit
without opening an empty picker, and multi-select is disabled.

It also registers quiet `chpwd` and conservative `preexec` hooks. Hook functions
use `emulate -L zsh` and do not use `emulate sh`.

## Commands

Track paths:

```sh
waymark add --kind auto -- ./README.md "$PWD"
waymark add --kind file -- ./README.md
waymark add --kind dir -- "$PWD"
```

Query paths:

```sh
waymark query --kind any -- config
waymark query --kind file --best -- zshrc
waymark query --kind dir --score -- project
waymark query --kind any --limit 50 -- work config
```

Import fasd data:

```sh
waymark import fasd --dry-run
waymark import fasd
waymark import fasd --from ~/.fasd --keep-missing
```

Inspect the local database:

```sh
waymark doctor
waymark dump --format json
```

Delete or prune records:

```sh
waymark delete -- /path/to/item
waymark prune --missing
```

## Fasd Import

Waymark imports fasd records in this format:

```text
/path/to/item|rank|unix_timestamp
```

Default import locations are checked in order:

- `$XDG_CACHE_HOME/fasd`
- `$HOME/.cache/fasd`
- `$HOME/.fasd`

Missing paths are skipped by default. Pass `--keep-missing` to retain them as
`unknown` records. Pass `--dry-run` to parse and summarize without writing to
the database.

## Comma Completion

The zsh integration supports the fasd-style forms used by the MVP:

```text
,query
f,query
d,query
query,,,
query,,f
query,,d
```

Completion candidates are queried from the Waymark database and added through
zsh completion APIs.

## Database

The database defaults to:

```text
$XDG_DATA_HOME/waymark/waymark.db
```

If `XDG_DATA_HOME` is unset, Waymark uses:

```text
$HOME/.local/share/waymark/waymark.db
```

Set `WAYMARK_DB=/path/to/waymark.db` to override the location.

## Development

Run the verification suite:

```sh
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --all-features
```

The test suite covers ranking, query behavior, fasd import, zsh integration,
hook option-state safety, and comma completion dispatch.
