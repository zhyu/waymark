use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

#[test]
fn cli_supports_json_dump_delete_and_prune_missing() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("waymark.db");
    let file = temp.path().join("config file.txt");
    fs::write(&file, "content").expect("write file");

    assert_success(run(&db_path, ["add", "--kind", "file", "--"], [&file]));

    let scored = run(
        &db_path,
        ["query", "--kind", "file", "--best", "--score", "--"],
        [&file],
    );
    assert_success(scored);

    let json = run(
        &db_path,
        ["query", "--kind", "file", "--format", "json", "--"],
        [&file],
    );
    let value = stdout_json(assert_success(json));
    assert_eq!(value.as_array().expect("query json array").len(), 1);

    let dump = run(&db_path, ["dump", "--format", "json"], []);
    let value = stdout_json(assert_success(dump));
    assert_eq!(value.as_array().expect("dump json array").len(), 1);

    assert_success(run(&db_path, ["delete", "--"], [&file]));
    let missing_query = run(
        &db_path,
        ["query", "--kind", "file", "--best", "--"],
        [&file],
    );
    assert!(!missing_query.status.success());

    let missing = temp.path().join("missing.txt");
    let fasd = temp.path().join("fasd");
    fs::write(&fasd, format!("{}|2.0|1710000000\n", missing.display())).expect("write fasd");
    assert_success(run(
        &db_path,
        ["import", "fasd", "--keep-missing", "--from"],
        [&fasd],
    ));

    let dump = run(&db_path, ["dump", "--format", "json"], []);
    let value = stdout_json(assert_success(dump));
    assert_eq!(value.as_array().expect("dump with missing").len(), 1);

    assert_success(run(&db_path, ["prune", "--missing"], []));
    let dump = run(&db_path, ["dump", "--format", "json"], []);
    let value = stdout_json(assert_success(dump));
    assert_eq!(value.as_array().expect("dump after prune").len(), 0);
}

#[test]
fn cli_json_represents_newline_and_unicode_paths() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("waymark.db");
    let file = temp.path().join("unicodé quote'\nfile.txt");
    fs::write(&file, "content").expect("write file");

    assert_success(run(&db_path, ["add", "--kind", "file", "--"], [&file]));

    let json = run(
        &db_path,
        ["query", "--kind", "file", "--format", "json", "--", "file"],
        [],
    );
    let value = stdout_json(assert_success(json));
    let results = value.as_array().expect("query json array");
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["entry"]["path"].as_str().expect("json path"),
        file.to_string_lossy()
    );
}

fn run<const N: usize, const M: usize>(
    db_path: &Path,
    args: [&str; N],
    paths: [&Path; M],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_waymark"));
    command.args(args);
    command.args(paths);
    command.env("WAYMARK_DB", db_path);
    command.output().expect("run waymark")
}

fn assert_success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn stdout_json(output: Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout json")
}
