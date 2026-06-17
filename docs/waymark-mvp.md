# Building a Modern Fasd Replacement

## Summary

This document describes a new command-line tool that replaces the parts of
`fasd` that matter most for interactive zsh usage:

- track both files and directories
- rank paths by frequency and recency
- provide fast query commands
- provide zsh aliases and completions comparable to `fasd`
- support fasd data import as a required migration path
- avoid unsafe shell-state behavior inside zsh hooks

The implementation is expected to live in a separate repository. This document
is self contained so it can be copied into that repository as an initial design
brief.

The project name used below is `Waymark`, with `waymark` as the binary name.

## Motivation

`fasd` remains useful because it is more than a directory jumper. It tracks
both recently and frequently used directories and files, then exposes those
paths through short shell commands and completion. In particular, fasd supports
workflows such as:

```sh
z proj          # jump to a ranked directory
f config        # print a ranked file
vim ,config<Tab>
cd d,repo<Tab>
```

Modern alternatives such as `zoxide`, `z.lua`, `zsh-z`, `z`, and `autojump`
are good directory jumpers, but they generally do not replace fasd's file
tracking and comma-completion workflows.

The current fasd codebase is also old shell code. Some forks add useful fixes,
but the design still has sharp edges. One concrete problem is zsh emulation
state. Fasd uses sh emulation internally so it can behave like a portable shell
script. In zsh, `emulate sh` enables options such as `KSH_ARRAYS`. If fasd
triggers another zsh hook while that emulation state is active, unrelated hook
code can run with different parsing rules than expected.

For example, this common zsh idiom works in native zsh mode:

```zsh
(( $+functions[deactivate] )) && deactivate
```

Under sh/ksh array parsing, it can be parsed as if it were:

```zsh
(( 1[deactivate] ))
```

which fails with:

```text
bad output format specification
```

The replacement should be designed so hook code is small, zsh-native, quiet,
and explicit about shell state. The core path database and ranking logic should
live in a normal compiled CLI rather than in a large shell script.

## Goals

The tool should provide fasd-like behavior for interactive zsh users while
being easier to maintain and safer to run from shell hooks.

Required goals:

- zsh-first interactive experience
- track files and directories
- rank paths using frequency and recency
- support exact, fuzzy, and multi-token path queries
- provide fast non-interactive query commands
- provide fzf-backed interactive selection
- provide fasd-like command aliases
- provide fasd-like comma completion for ordinary command arguments
- import existing fasd data
- preserve enough fasd behavior that migration is low friction
- keep hook execution quiet and bounded in latency
- avoid global shell-option changes from zsh hooks
- store data in a robust local database
- support safe path handling for spaces, quotes, newlines, unicode, and symlinks

Non-goals for v1:

- first-class bash or fish integration
- network sync
- shell history replacement
- full command-line parser for every possible shell command
- perfect reproduction of fasd's ranking order in every edge case
- daemon-only architecture

Future goals:

- bash and fish integrations
- importers for `zoxide`, `z`, `z.lua`, and `autojump`
- package manager releases
- optional background pruning or indexing
- optional shell-history integration

## Target User Experience

The primary user is someone who works in zsh, jumps between many project
directories, and opens recently used files from the shell.

The tool should make these flows natural:

```sh
z dotfiles              # cd to the best matching directory
zz dotfiles             # choose a matching directory through fzf
d dotfiles              # print the best matching directory
f config                # print the best matching file
sf config               # choose a matching file through fzf
s config                # list scored file and directory matches
nvim ,zshrc<Tab>        # complete to a ranked matching file
cd d,dotfiles<Tab>      # complete to a ranked matching directory
open ,report<Tab>       # complete to a ranked matching file or directory
```

Commands should be predictable enough for scripting:

```sh
waymark query --kind file config
waymark query --kind dir dotfiles
waymark add --kind file ./README.md
waymark add --kind dir "$PWD"
waymark import fasd --dry-run
waymark import fasd
```

## Compatibility Requirements

### Fasd Data Import

Importing fasd data is a must-have feature.

Fasd data is commonly stored as text records with path, rank, and timestamp:

```text
/path/to/item|rank|unix_timestamp
```

The importer should support:

- default fasd data locations:
  - `$XDG_CACHE_HOME/fasd`
  - `$HOME/.cache/fasd`
  - `$HOME/.fasd`
- explicit import path:
  - `waymark import fasd --from /path/to/fasd-data`
- dry-run mode:
  - show parsed record count
  - show skipped record count
  - show file vs directory split
  - show missing paths
- merge mode:
  - preserve existing records by default
  - combine imported rank with existing rank
  - use latest timestamp as `last_accessed`
- strict mode:
  - fail on malformed records
- permissive mode:
  - skip malformed records and report them
- missing path handling:
  - skip missing paths by default
  - allow `--keep-missing` for users who want historical data retained
- kind inference:
  - use `stat` to classify each imported path as file or directory
  - store unknown kind only when `--keep-missing` is set

The import command should never mutate data until parsing and validation have
completed successfully.

Example:

```sh
waymark import fasd --dry-run
waymark import fasd --merge
waymark import fasd --from ~/.fasd --keep-missing
```

### Fasd Command Compatibility

The shell integration should provide familiar aliases or functions:

```sh
a      # query files and directories
s      # show scored results
sd     # show scored directory results
sf     # show scored file results
d      # print best directory
f      # print best file
z      # cd to best directory
zz     # interactive cd to directory
```

These commands do not need to reproduce every fasd flag in v1, but common usage
should work. The compatibility layer can translate familiar aliases to native
`waymark` commands.

Potential mapping:

```sh
a query        -> waymark query --kind any query
s query        -> waymark query --kind any --score query
sd query       -> waymark query --kind dir --score query
sf query       -> waymark query --kind file --score query
d query        -> waymark query --kind dir --best query
f query        -> waymark query --kind file --best query
z query        -> cd "$(waymark query --kind dir --best query)"
zz query       -> cd "$(waymark query --kind dir --interactive query)"
```

### Fasd Completion Compatibility

Comma completion is a must-have feature because it is one of fasd's strongest
interactive affordances.

The zsh integration should complete path-like words using ranked data:

```text
,query       complete file or directory
f,query      complete file
d,query      complete directory
query,,      complete file or directory
query,,f     complete file
query,,d     complete directory
```

The exact trigger grammar can be adjusted if needed, but the fasd forms above
should work for migration.

Completion should:

- use zsh completion APIs, not ad-hoc terminal output
- preserve spaces and special characters in paths
- offer menu selection
- avoid changing global zsh options
- avoid running slow filesystem scans
- avoid printing errors during completion
- support command-aware defaults when practical

For example, `cd ,repo<Tab>` should prefer directories, while
`nvim ,config<Tab>` should prefer files. Command-aware behavior is desirable,
but explicit prefixes like `d,repo` and `f,config` are the compatibility
baseline.

## Architecture

Use a small native CLI plus zsh integration code.

The CLI owns:

- database reads and writes
- ranking
- matching
- import
- pruning
- diagnostics
- stable machine-readable output

The zsh integration owns:

- aliases and shell functions
- hook registration
- completion widgets
- `cd` behavior
- fzf invocation
- shell quoting and command substitution boundaries

The zsh code should stay small enough to audit.

## Language Choice

Rust is the recommended implementation language.

Reasons:

- fast startup for hook-called commands
- reliable single-binary distribution
- strong path and string handling
- good SQLite support
- good CLI ecosystem
- memory safety without a runtime dependency
- practical cross-platform support

Go is also viable, especially for contributor familiarity, but SQLite support
usually means either CGO or a pure-Go SQLite implementation. For this tool,
startup behavior, database correctness, and low-level path handling are
important enough that Rust is a better default.

Recommended Rust crates:

- `clap` for CLI parsing
- `rusqlite` or `sqlite` for SQLite access
- `serde` and `serde_json` for dump/import formats
- `ignore` or `globset` for exclude rules
- `shell-words` or custom tested quoting helpers where needed
- `tempfile` for tests
- `assert_cmd` for integration tests

## Data Storage

Use SQLite as the primary store.

Default path:

```text
$XDG_DATA_HOME/waymark/waymark.db
```

Fallback:

```text
$HOME/.local/share/waymark/waymark.db
```

Environment override:

```text
WAYMARK_DB=/path/to/db
```

Initial schema:

```sql
create table entries (
  path text primary key not null,
  kind text not null check (kind in ('file', 'dir', 'unknown')),
  rank real not null default 1.0,
  access_count integer not null default 1,
  first_seen integer not null,
  last_accessed integer not null,
  last_seen integer not null,
  source text not null default 'native'
);

create index entries_kind_last_accessed
  on entries(kind, last_accessed);

create index entries_kind_rank
  on entries(kind, rank);
```

Optional later schema:

```sql
create table entry_aliases (
  alias text not null,
  path text not null references entries(path) on delete cascade,
  source text not null,
  primary key (alias, path)
);
```

Avoid over-designing indexing early. For a personal shell database with
thousands or tens of thousands of paths, a simple query plus in-process matching
may be faster and easier to tune than a complex full-text setup.

## Ranking

Ranking should combine:

- frequency: how often the path was used
- recency: how recently the path was used
- match quality: how well the query matches the path
- kind preference: file vs directory, depending on command context

The scoring model should be stable and explainable.

Candidate formula:

```text
score = match_score * frecency_score * kind_weight
```

Frecency can start with a fasd-like model:

```text
frecency_score = rank * recency_multiplier(last_accessed)
```

Example recency multipliers:

```text
last hour     6
last day      4
last week     2
older         1
```

When a path is accessed:

```text
rank = rank + 1 / rank
access_count = access_count + 1
last_accessed = now
last_seen = now
```

Periodic aging can prevent old records from dominating forever:

```text
rank = rank * 0.9
```

The exact constants can change, but tests should lock down expected ordering
for representative cases.

## Matching

The query matcher should support:

- case-sensitive exact substring match
- case-insensitive fallback
- multi-token ordered match
- fuzzy fallback
- basename boost
- path segment boundary boost
- exact basename boost

Example:

```sh
waymark query --kind file zsh rc
```

should match paths like:

```text
/Users/me/workspace/dotfiles/zsh/zshrc
/Users/me/.zshrc
```

Matching should return deterministic ordering. If scores tie, prefer:

1. more recent path
2. higher access count
3. shorter path
4. lexicographic path

## CLI Design

The binary should expose stable subcommands.

```sh
waymark init zsh
waymark add --kind file ./README.md
waymark add --kind dir "$PWD"
waymark query --kind any config
waymark query --kind file --best config
waymark query --kind dir --interactive repo
waymark import fasd
waymark prune
waymark doctor
waymark dump --format json
```

### `waymark init zsh`

Print zsh integration code to stdout:

```sh
eval "$(waymark init zsh)"
```

The generated code should:

- define shell functions and aliases
- register zsh hooks
- define completion functions
- set no global options unless explicitly required
- wrap functions with `emulate -L zsh`
- ignore hook errors
- write no output from hooks

### `waymark add`

Add one or more paths:

```sh
waymark add --kind auto -- ./file ./dir
waymark add --kind dir -- "$PWD"
waymark add --kind file -- "$file"
```

Behavior:

- normalize to absolute paths
- preserve symlink behavior by default
- optionally resolve symlinks with config
- skip missing paths unless explicitly allowed
- infer kind with filesystem metadata when `--kind auto`
- update rank and timestamps atomically

### `waymark query`

Query paths:

```sh
waymark query --kind any config
waymark query --kind file --best config
waymark query --kind dir --limit 20 repo
waymark query --kind any --score config
waymark query --kind dir --format json repo
```

Output modes:

- default: paths only, one per line
- `--score`: score and path
- `--best`: best path only
- `--format json`: machine-readable output
- `--interactive`: fzf selection

### `waymark import fasd`

Import fasd data:

```sh
waymark import fasd
waymark import fasd --dry-run
waymark import fasd --from ~/.fasd
waymark import fasd --keep-missing
```

This command is required for v1.

### `waymark doctor`

Report common setup problems:

- database path
- database readability and writability
- zsh version
- whether hooks are registered
- whether completion is installed
- fzf availability
- fasd data source detection
- number of records by kind
- records pointing to missing paths
- slow query warnings

## Zsh Integration

The zsh integration is the highest priority shell integration.

All generated zsh functions should start with:

```zsh
emulate -L zsh
```

If a helper needs specific options, set them locally after `emulate -L zsh`.
Do not run `emulate sh` in zsh integration code.

### Hooks

Use zsh hooks through `add-zsh-hook`.

Recommended hooks:

- `chpwd`: record the new current directory
- `preexec`: optionally parse the command about to run and record file args

Directory tracking should be always on by default:

```zsh
_waymark_chpwd() {
  emulate -L zsh
  [[ -o interactive ]] || return 0
  command waymark add --kind dir -- "$PWD" >/dev/null 2>&1 || true
}
```

File tracking should start conservative. A preexec hook can parse common file
opening commands, but should avoid pretending shell command lines are easy to
parse perfectly.

Initial allowlist:

```text
vim
nvim
vi
code
less
more
cat
bat
open
xdg-open
```

For v1, it is acceptable to track only existing path arguments passed directly
to these commands. More advanced shell parsing can come later.

Safety requirements:

- hooks must not print to stdout or stderr
- hooks must never abort the user's command
- hooks must not use `eval`
- hooks must use `command waymark` to avoid aliases/functions
- hooks must handle paths with spaces and quotes
- hooks must include a reentrancy guard
- hooks must avoid changing directories
- hooks must avoid global option changes

### Zsh Completion

Completion should use zsh's completion system.

Primary behavior:

- detect current word
- recognize fasd-style comma triggers
- call `waymark query` with the right kind
- add matches with `compadd`
- preserve menu selection

The completion code should be careful with path quoting. Paths should be added
as completion candidates, not eval'd into shell code.

Pseudo-shape:

```zsh
_waymark_complete_word() {
  emulate -L zsh
  local cur="${words[CURRENT]}"
  local kind="any"
  local query

  case "$cur" in
    f,*) kind="file"; query="${cur#f,}" ;;
    d,*) kind="dir"; query="${cur#d,}" ;;
    ,*)  kind="any"; query="${cur#,}" ;;
    *,,f) kind="file"; query="${cur%,,f}" ;;
    *,,d) kind="dir"; query="${cur%,,d}" ;;
    *,,)  kind="any"; query="${cur%,,}" ;;
    *) return 1 ;;
  esac

  local -a matches
  matches=("${(@f)$(command waymark query --kind "$kind" --limit 50 -- "$query" 2>/dev/null)}")
  (( ${#matches} )) || return 1
  compadd -U -V waymark -- "$matches[@]"
}
```

The real implementation should include tests for:

- spaces
- quotes
- brackets
- unicode
- paths beginning with `-`
- missing database
- no matches

### Zsh Aliases and Functions

Suggested generated zsh functions:

```zsh
waymark-z() {
  emulate -L zsh
  local dest
  dest="$(command waymark query --kind dir --best -- "$@")" || return
  [[ -n "$dest" ]] || return 1
  builtin cd -- "$dest"
}

waymark-zz() {
  emulate -L zsh
  local dest
  dest="$(command waymark query --kind dir --interactive -- "$@")" || return
  [[ -n "$dest" ]] || return 1
  builtin cd -- "$dest"
}
```

Aliases:

```zsh
alias z='waymark-z'
alias zz='waymark-zz'
alias f='waymark query --kind file --best --'
alias d='waymark query --kind dir --best --'
alias a='waymark query --kind any --best --'
alias s='waymark query --kind any --score --'
alias sf='waymark query --kind file --score --'
alias sd='waymark query --kind dir --score --'
```

## Configuration

Config path:

```text
$XDG_CONFIG_HOME/waymark/config.toml
```

Fallback:

```text
$HOME/.config/waymark/config.toml
```

Example:

```toml
[database]
path = ""

[tracking]
track_directories = true
track_files = true
resolve_symlinks = false
keep_missing = false

[tracking.preexec]
enabled = true
commands = ["vim", "nvim", "vi", "code", "less", "cat", "bat", "open"]

[query]
default_limit = 20
fuzzy = true
case_insensitive_fallback = true

[exclude]
paths = [
  "/tmp",
  "/private/tmp",
  "/var/folders",
  ".git",
  "node_modules",
  "target",
  ".cache",
]
```

Configuration should not be required for normal use.

## Privacy and Safety

The database may contain sensitive path names. The tool should:

- store data only locally
- never send telemetry
- make sync an explicit non-default feature if it ever exists
- support exclude patterns
- support pruning missing paths
- support deleting records
- support dumping records for inspection

Commands:

```sh
waymark delete -- /secret/path
waymark prune --missing
waymark dump --format json
```

## Performance Requirements

Hook use makes latency important.

Targets:

- `waymark add "$PWD"`: under 20 ms for normal database sizes
- `waymark query --best`: under 30 ms for 100k records
- zsh hook stdout/stderr: always empty
- zsh hook failure: ignored
- concurrent shell writes: safe

Use SQLite transactions and reasonable pragmas. Avoid holding write locks for
long operations. Import can be slower, but normal hooks should be fast.

Benchmark sizes:

- 1k entries
- 10k entries
- 100k entries
- 1M entries

## Testing Strategy

Unit tests:

- ranking
- matching
- path normalization
- fasd import parsing
- merge behavior
- missing path behavior
- config loading
- exclude matching

Integration tests:

- spawn real zsh
- load `eval "$(waymark init zsh)"`
- verify `chpwd` tracking
- verify preexec allowlist tracking
- verify aliases
- verify comma completion helpers
- verify no hook stdout/stderr
- verify hooks preserve zsh option state

Specific regression test for zsh option leakage:

```zsh
victim() {
  emulate -L zsh
  (( $+functions[deactivate] )) && deactivate
}
```

The test should prove that running waymark hooks before `victim` does not leave
`KSH_ARRAYS` enabled in the victim context.

Import fixture tests:

```text
/tmp/project|12.5|1710000000
/tmp/project/README.md|3.25|1710000100
malformed line
```

Expected behavior:

- parse valid records
- classify file vs directory
- report malformed line
- support strict and permissive modes
- preserve rank and timestamp

Shell quoting tests:

- spaces
- tabs
- single quotes
- double quotes
- brackets
- glob characters
- leading dash
- unicode
- newlines if supported

## Milestones

### Milestone 1: Core CLI

- initialize Rust project
- implement SQLite schema and migrations
- implement `add`
- implement `query`
- implement basic ranking
- implement path normalization
- implement JSON and plain output
- add unit tests

### Milestone 2: Fasd Import

- detect fasd data paths
- parse fasd records
- dry-run summary
- import and merge
- classify file vs directory
- handle missing paths
- add import fixtures

This milestone is required before dogfooding.

### Milestone 3: Zsh Integration

- implement `waymark init zsh`
- add `z`, `zz`, `f`, `d`, `a`, `s`, `sf`, `sd`
- add `chpwd` tracking
- add conservative `preexec` file tracking
- add reentrancy guard
- add integration tests with zsh

### Milestone 4: Completion

- implement fasd-style comma completion
- support file, directory, and any-kind queries
- support menu selection
- test special path characters
- test missing database and no-match behavior

### Milestone 5: Interactive Selection

- add fzf integration for `zz`, `sf`, `sd`, and `s`
- gracefully fall back when fzf is unavailable
- add `doctor` checks

### Milestone 6: Hardening

- benchmark large databases
- add pruning
- add delete command
- add config file
- add install instructions
- prepare releases

## Open Questions

- Should missing imported fasd paths be stored as `unknown` by default, or only
  with `--keep-missing`?
- Should symlinks be preserved by default to match shell-visible paths, or
  resolved to avoid duplicates?
- How close should the ranking algorithm stay to fasd's exact ordering?
- Should comma completion be globally installed into zsh completion, or bound
  to a widget users opt into?
- Should file tracking parse only allowlisted commands, or should users be able
  to opt into broader preexec parsing?
- Should `a`, `f`, and `d` be aliases or functions? Functions are easier to
  make robust, but aliases feel closer to fasd.

## Recommended v1 Definition

v1 is complete when this works reliably in zsh:

```sh
eval "$(waymark init zsh)"
waymark import fasd
z some-project
f some-file
s query
nvim ,query<Tab>
cd d,query<Tab>
zz query
sf query
```

and when these properties are true:

- fasd data import is tested
- both files and directories are tracked
- zsh hooks are quiet
- zsh hooks preserve option state
- paths with spaces and quotes work
- query latency is acceptable for at least 100k records
- the user can inspect, prune, and delete records

That v1 would cover the practical reason to keep using fasd while removing the
old shell-script internals that make maintenance and hook interactions fragile.
