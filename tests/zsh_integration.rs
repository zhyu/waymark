use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn generated_zsh_code_has_required_integration_points() {
    let script = waymark::zsh::init_script();

    assert!(script.contains("emulate -L zsh"));
    assert!(!script.contains("emulate sh"));
    assert!(!script.contains("eval"));

    for name in ["z", "zz", "f", "v", "vv", "d", "a", "s", "sf", "sd"] {
        assert!(
            script.contains(&format!("{name}() {{")),
            "missing {name} function"
        );
    }

    assert!(script.contains("command waymark add --kind dir -- \"$PWD\" >/dev/null 2>&1"));
    assert!(
        script.contains("command waymark add --kind file -- \"$waymark_files[@]\" >/dev/null 2>&1")
    );
    assert!(script.contains("add-zsh-hook chpwd _waymark_chpwd"));
    assert!(script.contains("add-zsh-hook preexec _waymark_preexec"));
    assert!(
        script.contains(
            "waymark_output=\"$(command waymark query --kind \"$kind\" --limit \"$waymark_limit\" -- \"$query_tokens[@]\" 2>/dev/null)\" || return 1"
        )
    );
    assert!(script.contains("[[ -n \"$waymark_output\" ]] || return 1"));
    assert!(script.contains("compadd -U -V waymark -- \"$matches[@]\""));
    assert!(script.contains("zstyle ':completion:*' completer _waymark_comma_complete"));
    assert!(script.contains("compdef _waymark_comma_complete waymark z zz f v vv d a s sf sd"));

    for pattern in ["f,*)", "d,*)", ",*)", "*,,,)", "*,,f)", "*,,d)", "*,,)"] {
        assert!(
            script.contains(pattern),
            "missing completion pattern {pattern}"
        );
    }
}

#[test]
fn init_zsh_command_prints_generated_script() {
    let output = Command::new(env!("CARGO_BIN_EXE_waymark"))
        .args(["init", "zsh"])
        .output()
        .expect("run waymark init zsh");

    assert!(
        output.status.success(),
        "status={:?}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is utf-8"),
        waymark::zsh::init_script()
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn zsh_v_and_vv_open_waymark_files_in_editor_when_zsh_is_available() {
    if !zsh_is_available() {
        eprintln!("skipping zsh editor helper test because zsh is not available");
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir(&bin_dir).expect("create bin dir");

    let init_path = temp.path().join("waymark.zsh");
    let query_log = temp.path().join("queries.log");
    let output_path = temp.path().join("query-output.txt");
    let editor_log = temp.path().join("editor.log");
    let editor_path = bin_dir.join("editor");
    let file = temp.path().join("file with spaces.txt");
    fs::write(&init_path, waymark::zsh::init_script()).expect("write init script");
    fs::write(&output_path, format!("{}\n", file.display())).expect("write query output");
    write_stub_waymark(&bin_dir.join("waymark"), &query_log);
    write_stub_editor(&editor_path);

    let zsh = format!(
        r#"
source {init}
EDITOR={editor}
v alpha || exit 50
vv beta || exit 51
"#,
        init = zsh_quote(&init_path),
        editor = zsh_quote(&editor_path),
    );

    let mut command = Command::new("zsh");
    command.arg("-fc").arg(zsh);
    command.env("WAYMARK_QUERY_OUTPUT_FILE", &output_path);
    command.env("WAYMARK_QUERY_LOG", &query_log);
    command.env("WAYMARK_EDITOR_LOG", &editor_log);
    command.env("PATH", path_with_front(&bin_dir));
    let output = command.output().expect("run zsh editor helper check");
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let queries = fs::read_to_string(&query_log).expect("read query log");
    assert_eq!(queries, ["file\talpha\n", "file\tbeta\n"].concat());

    let editor_calls = fs::read_to_string(&editor_log).expect("read editor log");
    assert_eq!(
        editor_calls,
        [
            format!("--\t{}\n", file.display()),
            format!("--\t{}\n", file.display()),
        ]
        .concat()
    );
}

#[test]
fn zsh_hooks_do_not_leak_ksh_arrays_or_output_when_zsh_is_available() {
    if !zsh_is_available() {
        eprintln!("skipping zsh regression test because zsh is not available");
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    let init_path = temp.path().join("waymark.zsh");
    let stdout_path = temp.path().join("hook.stdout");
    let stderr_path = temp.path().join("hook.stderr");
    fs::write(&init_path, waymark::zsh::init_script()).expect("write init script");

    let zsh = format!(
        r#"
source {init}
: > {hook_stdout}
: > {hook_stderr}

unsetopt KSH_ARRAYS
_waymark_chpwd >> {hook_stdout} 2>> {hook_stderr}
_waymark_preexec "vim ./file with spaces.txt" >> {hook_stdout} 2>> {hook_stderr}
[[ ! -o KSH_ARRAYS ]] || exit 10

setopt KSH_ARRAYS
_waymark_chpwd >> {hook_stdout} 2>> {hook_stderr}
[[ -o KSH_ARRAYS ]] || exit 11

unsetopt KSH_ARRAYS
deactivate() {{ return 0 }}
victim() {{
  (( $+functions[deactivate] )) && deactivate
}}
victim || exit 12

[[ ! -s {hook_stdout} ]] || exit 13
[[ ! -s {hook_stderr} ]] || exit 14
"#,
        init = zsh_quote(&init_path),
        hook_stdout = zsh_quote(&stdout_path),
        hook_stderr = zsh_quote(&stderr_path),
    );

    run_zsh_interactive(&zsh);
}

#[test]
fn zsh_comma_completion_dispatches_supported_forms_when_zsh_is_available() {
    if !zsh_is_available() {
        eprintln!("skipping zsh completion test because zsh is not available");
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir(&bin_dir).expect("create bin dir");

    let init_path = temp.path().join("waymark.zsh");
    let query_log = temp.path().join("queries.log");
    let compadd_log = temp.path().join("compadd.log");
    fs::write(&init_path, waymark::zsh::init_script()).expect("write init script");
    write_stub_waymark(&bin_dir.join("waymark"), &query_log);

    let zsh = format!(
        r#"
source {init}

compadd() {{
  print -r -- "$*" >> {compadd_log}
}}

run_case() {{
  words=("$1")
  CURRENT=1
  _waymark_comma_complete || return 1
}}

run_case ",alpha"
run_case "f,beta"
run_case "d,gamma"
run_case "delta,,"
run_case "epsilon,,,"
run_case "zeta,,f"
run_case "eta,,d"
run_case ",alpha,beta"
run_case ",alpha,beta,"
run_case "d,web,root"
run_case "theta,phi,,f"
"#,
        init = zsh_quote(&init_path),
        compadd_log = zsh_quote(&compadd_log),
    );

    let mut command = Command::new("zsh");
    command.arg("-fc").arg(zsh);
    command.env("WAYMARK_QUERY_LOG", &query_log);
    command.env("PATH", path_with_front(&bin_dir));
    let output = command.output().expect("run zsh completion check");
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let queries = fs::read_to_string(&query_log).expect("read query log");
    assert_eq!(
        queries,
        [
            "any\talpha\n",
            "file\tbeta\n",
            "dir\tgamma\n",
            "any\tdelta \n",
            "any\tepsilon \n",
            "file\tzeta \n",
            "dir\teta \n",
            "any\talpha beta\n",
            "any\talpha beta \n",
            "dir\tweb root\n",
            "file\ttheta phi \n",
        ]
        .concat()
    );

    let compadd_calls = fs::read_to_string(&compadd_log).expect("read compadd log");
    assert_eq!(compadd_calls.lines().count(), 11);
}

#[test]
fn zsh_comma_completion_requests_menu_for_multiple_matches_when_zsh_is_available() {
    if !zsh_is_available() {
        eprintln!("skipping zsh completion menu test because zsh is not available");
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir(&bin_dir).expect("create bin dir");

    let init_path = temp.path().join("waymark.zsh");
    let query_log = temp.path().join("queries.log");
    fs::write(&init_path, waymark::zsh::init_script()).expect("write init script");
    write_stub_waymark(&bin_dir.join("waymark"), &query_log);

    let zsh = format!(
        r#"
source {init}
typeset -A compstate

compadd() {{
  return 0
}}

words=(",alpha")
CURRENT=1
_waymark_comma_complete || exit 40
[[ "${{compstate[insert]}}" = menu ]] || exit 41
[[ "${{compstate[list]}}" = list ]] || exit 42
"#,
        init = zsh_quote(&init_path),
    );

    let mut command = Command::new("zsh");
    command.arg("-fc").arg(zsh);
    command.env("WAYMARK_QUERY_LOG", &query_log);
    command.env("PATH", path_with_front(&bin_dir));
    let output = command.output().expect("run zsh completion menu check");
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn zsh_comma_completion_uses_default_and_configured_limit_when_zsh_is_available() {
    if !zsh_is_available() {
        eprintln!("skipping zsh completion limit test because zsh is not available");
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir(&bin_dir).expect("create bin dir");

    let init_path = temp.path().join("waymark.zsh");
    let query_log = temp.path().join("queries.log");
    let limit_log = temp.path().join("limits.log");
    fs::write(&init_path, waymark::zsh::init_script()).expect("write init script");
    write_stub_waymark(&bin_dir.join("waymark"), &query_log);

    let zsh = format!(
        r#"
source {init}

compadd() {{
  return 0
}}

run_case() {{
  words=("$1")
  CURRENT=1
  _waymark_comma_complete || return 1
}}

run_case ",alpha"
WAYMARK_COMPLETION_LIMIT=7
run_case ",beta"
WAYMARK_COMPLETION_LIMIT=bogus
run_case ",gamma"
"#,
        init = zsh_quote(&init_path),
    );

    let mut command = Command::new("zsh");
    command.arg("-fc").arg(zsh);
    command.env("WAYMARK_QUERY_LOG", &query_log);
    command.env("WAYMARK_QUERY_LIMIT_LOG", &limit_log);
    command.env("PATH", path_with_front(&bin_dir));
    let output = command.output().expect("run zsh completion limit check");
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let limits = fs::read_to_string(&limit_log).expect("read limit log");
    assert_eq!(limits, ["1\n", "7\n", "1\n"].concat());
}

#[test]
fn zsh_completion_returns_failure_for_empty_query_output_when_zsh_is_available() {
    if !zsh_is_available() {
        eprintln!("skipping zsh empty-completion test because zsh is not available");
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir(&bin_dir).expect("create bin dir");

    let init_path = temp.path().join("waymark.zsh");
    let query_log = temp.path().join("queries.log");
    let compadd_log = temp.path().join("compadd.log");
    fs::write(&init_path, waymark::zsh::init_script()).expect("write init script");
    write_stub_waymark(&bin_dir.join("waymark"), &query_log);

    let zsh = format!(
        r#"
source {init}

compadd() {{
  print -r -- "$*" >> {compadd_log}
}}

words=(",missing")
CURRENT=1
if _waymark_comma_complete; then
  exit 30
fi
[[ ! -s {compadd_log} ]] || exit 31
"#,
        init = zsh_quote(&init_path),
        compadd_log = zsh_quote(&compadd_log),
    );

    let mut command = Command::new("zsh");
    command.arg("-fc").arg(zsh);
    command.env("WAYMARK_EMPTY_QUERY", "1");
    command.env("WAYMARK_QUERY_LOG", &query_log);
    command.env("PATH", path_with_front(&bin_dir));
    let output = command.output().expect("run zsh empty-completion check");
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn zsh_completion_preserves_space_and_quote_candidates_when_zsh_is_available() {
    if !zsh_is_available() {
        eprintln!("skipping zsh special-candidate completion test because zsh is not available");
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir(&bin_dir).expect("create bin dir");

    let init_path = temp.path().join("waymark.zsh");
    let query_log = temp.path().join("queries.log");
    let output_path = temp.path().join("query-output.txt");
    let compadd_log = temp.path().join("compadd.log");
    let spaced = temp.path().join("space and 'quote'.txt");
    fs::write(&init_path, waymark::zsh::init_script()).expect("write init script");
    fs::write(&output_path, format!("{}\n", spaced.display())).expect("write query output");
    write_stub_waymark(&bin_dir.join("waymark"), &query_log);

    let zsh = format!(
        r#"
source {init}

compadd() {{
  local waymark_arg
  for waymark_arg in "$@"; do
    print -r -- "$waymark_arg" >> {compadd_log}
  done
}}

words=(",quote")
CURRENT=1
_waymark_comma_complete || exit 40
"#,
        init = zsh_quote(&init_path),
        compadd_log = zsh_quote(&compadd_log),
    );

    let mut command = Command::new("zsh");
    command.arg("-fc").arg(zsh);
    command.env("WAYMARK_QUERY_OUTPUT_FILE", &output_path);
    command.env("WAYMARK_QUERY_LOG", &query_log);
    command.env("PATH", path_with_front(&bin_dir));
    let output = command.output().expect("run zsh special-candidate check");
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let compadd_calls = fs::read_to_string(&compadd_log).expect("read compadd log");
    assert!(
        compadd_calls
            .lines()
            .any(|line| line == spaced.to_string_lossy()),
        "compadd log was {compadd_calls:?}"
    );
}

#[test]
fn zsh_completion_preserves_existing_completer_chain_when_zsh_is_available() {
    if !zsh_is_available() {
        eprintln!("skipping zsh completer-chain test because zsh is not available");
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    let init_path = temp.path().join("waymark.zsh");
    fs::write(&init_path, waymark::zsh::init_script()).expect("write init script");

    let zsh = format!(
        r#"
zstyle ':completion:*' completer _complete _ignored
source {init}
typeset -a got
zstyle -a ':completion:*' completer got || exit 20
[[ "$got[1]" = "_waymark_comma_complete" ]] || exit 21
[[ "$got[2]" = "_complete" ]] || exit 22
[[ "$got[3]" = "_ignored" ]] || exit 23
"#,
        init = zsh_quote(&init_path),
    );

    run_zsh(&zsh);
}

#[test]
fn zsh_preexec_tracks_leading_dash_paths_after_double_dash_when_zsh_is_available() {
    if !zsh_is_available() {
        eprintln!("skipping zsh leading-dash preexec test because zsh is not available");
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir(&bin_dir).expect("create bin dir");

    let init_path = temp.path().join("waymark.zsh");
    let query_log = temp.path().join("queries.log");
    let add_log = temp.path().join("adds.log");
    let leading = temp.path().join("-leading.txt");
    fs::write(&init_path, waymark::zsh::init_script()).expect("write init script");
    fs::write(&leading, "content").expect("write leading dash file");
    write_stub_waymark(&bin_dir.join("waymark"), &query_log);

    let zsh = format!(
        r#"
cd {work}
source {init}
_waymark_preexec "vim -- -leading.txt"
"#,
        work = zsh_quote(temp.path()),
        init = zsh_quote(&init_path),
    );

    let mut command = Command::new("zsh");
    command.arg("-fic").arg(zsh);
    command.env("WAYMARK_QUERY_LOG", &query_log);
    command.env("WAYMARK_ADD_LOG", &add_log);
    command.env("PATH", path_with_front(&bin_dir));
    let output = command.output().expect("run zsh preexec check");
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let adds = fs::read_to_string(&add_log).expect("read add log");
    assert!(adds.contains("\t-leading.txt\n"), "add log was {adds:?}");
}

#[test]
fn zsh_preexec_expands_current_user_tilde_when_zsh_is_available() {
    if !zsh_is_available() {
        eprintln!("skipping zsh tilde preexec test because zsh is not available");
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    let home = temp.path().join("home");
    fs::create_dir(&bin_dir).expect("create bin dir");
    fs::create_dir(&home).expect("create home dir");

    let init_path = temp.path().join("waymark.zsh");
    let query_log = temp.path().join("queries.log");
    let add_log = temp.path().join("adds.log");
    let zshrc = home.join(".zshrc");
    fs::write(&init_path, waymark::zsh::init_script()).expect("write init script");
    fs::write(&zshrc, "content").expect("write zshrc");
    write_stub_waymark(&bin_dir.join("waymark"), &query_log);

    let zsh = format!(
        r#"
source {init}
_waymark_preexec "vim ~/.zshrc"
"#,
        init = zsh_quote(&init_path),
    );

    let mut command = Command::new("zsh");
    command.arg("-fic").arg(zsh);
    command.env("HOME", &home);
    command.env("WAYMARK_QUERY_LOG", &query_log);
    command.env("WAYMARK_ADD_LOG", &add_log);
    command.env("PATH", path_with_front(&bin_dir));
    let output = command.output().expect("run zsh tilde preexec check");
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let adds = fs::read_to_string(&add_log).expect("read add log");
    assert!(
        adds.contains(&format!("\t{}\n", zshrc.display())),
        "add log was {adds:?}"
    );
}

fn zsh_is_available() -> bool {
    Command::new("zsh")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_zsh(script: &str) {
    let output = Command::new("zsh")
        .arg("-fc")
        .arg(script)
        .output()
        .expect("run zsh");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_zsh_interactive(script: &str) {
    let output = Command::new("zsh")
        .arg("-fic")
        .arg(script)
        .output()
        .expect("run interactive zsh");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_stub_waymark(path: &Path, query_log: &Path) {
    let script = r#"#!/bin/sh
if [ "$1" = "query" ]; then
  shift
  kind=
  limit=
  query=
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --kind)
        kind="$2"
        shift 2
        ;;
      --limit)
        limit="$2"
        shift 2
        ;;
      --)
        shift
        query="$*"
        break
        ;;
      *)
        shift
        ;;
    esac
  done
  printf '%s\t%s\n' "$kind" "$query" >> "$WAYMARK_QUERY_LOG"
  if [ -n "$WAYMARK_QUERY_LIMIT_LOG" ]; then
    printf '%s\n' "$limit" >> "$WAYMARK_QUERY_LIMIT_LOG"
  fi
  if [ -n "$WAYMARK_EMPTY_QUERY" ]; then
    exit 0
  fi
  if [ -n "$WAYMARK_QUERY_OUTPUT_FILE" ]; then
    cat "$WAYMARK_QUERY_OUTPUT_FILE"
  else
    printf '/tmp/waymark match one\n/tmp/waymark match two\n'
  fi
fi
if [ "$1" = "add" ] && [ -n "$WAYMARK_ADD_LOG" ]; then
  printf 'add' >> "$WAYMARK_ADD_LOG"
  for arg in "$@"; do
    printf '\t%s' "$arg" >> "$WAYMARK_ADD_LOG"
  done
  printf '\n' >> "$WAYMARK_ADD_LOG"
fi
exit 0
"#;
    fs::write(path, script).expect("write stub waymark");
    let mut permissions = fs::metadata(path).expect("stub metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod stub waymark");
    fs::write(query_log, "").expect("create query log");
}

fn write_stub_editor(path: &Path) {
    let script = r#"#!/bin/sh
first=1
for arg in "$@"; do
  if [ "$first" = 1 ]; then
    first=0
  else
    printf '\t' >> "$WAYMARK_EDITOR_LOG"
  fi
  printf '%s' "$arg" >> "$WAYMARK_EDITOR_LOG"
done
printf '\n' >> "$WAYMARK_EDITOR_LOG"
exit 0
"#;
    fs::write(path, script).expect("write stub editor");
    let mut permissions = fs::metadata(path).expect("editor metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod stub editor");
}

fn path_with_front(dir: &Path) -> String {
    let original = env::var_os("PATH").unwrap_or_default();
    format!("{}:{}", dir.display(), original.to_string_lossy())
}

fn zsh_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', r#"'\''"#))
}
