use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use tempfile::TempDir;
use waymark::fasd::{FasdImportOptions, import_fasd};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn imports_valid_records_from_default_xdg_location() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let cache_home = temp.path().join("xdg-cache");
    let home = temp.path().join("home");
    let db_path = temp.path().join("waymark.db");
    fs::create_dir_all(&cache_home).unwrap();
    fs::create_dir_all(&home).unwrap();
    let _env = EnvGuard::set(&[
        ("XDG_CACHE_HOME", cache_home.as_path()),
        ("HOME", home.as_path()),
        ("WAYMARK_DB", db_path.as_path()),
    ]);

    let project = temp.path().join("project");
    let file = project.join("read|me.txt");
    fs::create_dir_all(&project).unwrap();
    fs::write(&file, "hello").unwrap();
    write_fixture(
        &cache_home.join("fasd"),
        &format!(
            "{}|12.5|1710000000\n{}|3.25|1710000100\n",
            project.display(),
            file.display()
        ),
    );

    let summary = import_fasd(None, FasdImportOptions::default()).unwrap();

    assert_eq!(summary.parsed, 2);
    assert_eq!(summary.imported, 2);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.malformed, 0);
    assert_eq!(summary.missing, 0);
    assert_eq!(summary.files, 1);
    assert_eq!(summary.dirs, 1);
    assert_eq!(summary.unknown, 0);

    let project_entry = fetch_entry(&db_path, &project).unwrap();
    assert_eq!(project_entry.kind, "dir");
    assert_float_eq(project_entry.rank, 12.5);
    assert_eq!(project_entry.last_accessed, 1710000000);
    assert_eq!(project_entry.source, "fasd");

    let file_entry = fetch_entry(&db_path, &file).unwrap();
    assert_eq!(file_entry.kind, "file");
    assert_float_eq(file_entry.rank, 3.25);
    assert_eq!(file_entry.last_accessed, 1710000100);
}

#[test]
fn imports_valid_records_from_home_cache_when_xdg_location_is_missing() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let db_path = temp.path().join("waymark.db");
    fs::create_dir_all(home.join(".cache")).unwrap();
    let _env = EnvGuard::set_and_remove(
        &[("HOME", home.as_path()), ("WAYMARK_DB", db_path.as_path())],
        &["XDG_CACHE_HOME"],
    );

    let file = temp.path().join("from-home-cache.txt");
    fs::write(&file, "hello").unwrap();
    write_fixture(
        &home.join(".cache").join("fasd"),
        &format!("{}|4.5|1710000200\n", file.display()),
    );

    let summary = import_fasd(None, FasdImportOptions::default()).unwrap();

    assert_eq!(summary.parsed, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.files, 1);
    assert_eq!(
        fetch_entry(&db_path, &file).unwrap().last_accessed,
        1710000200
    );
}

#[test]
fn imports_valid_records_from_home_dot_fasd_when_cache_location_is_missing() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let db_path = temp.path().join("waymark.db");
    fs::create_dir_all(&home).unwrap();
    let _env = EnvGuard::set_and_remove(
        &[("HOME", home.as_path()), ("WAYMARK_DB", db_path.as_path())],
        &["XDG_CACHE_HOME"],
    );

    let file = temp.path().join("from-dot-fasd.txt");
    fs::write(&file, "hello").unwrap();
    write_fixture(
        &home.join(".fasd"),
        &format!("{}|5.5|1710000300\n", file.display()),
    );

    let summary = import_fasd(None, FasdImportOptions::default()).unwrap();

    assert_eq!(summary.parsed, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.files, 1);
    assert_eq!(
        fetch_entry(&db_path, &file).unwrap().last_accessed,
        1710000300
    );
}

#[test]
fn skips_malformed_records_permissively() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("waymark.db");
    let _env = EnvGuard::set(&[("WAYMARK_DB", db_path.as_path())]);

    let file = temp.path().join("valid.txt");
    fs::write(&file, "hello").unwrap();
    let fasd_path = temp.path().join("fasd");
    write_fixture(
        &fasd_path,
        &format!(
            "{}|4.0|1710000000\nmalformed\n{}|bad-rank|1710000001\n",
            file.display(),
            file.display()
        ),
    );

    let summary = import_fasd(Some(&fasd_path), FasdImportOptions::default()).unwrap();

    assert_eq!(summary.parsed, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 2);
    assert_eq!(summary.malformed, 2);
    assert_eq!(summary.files, 1);
    assert!(fetch_entry(&db_path, &file).is_some());
}

#[test]
fn strict_malformed_failure_does_not_mutate() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("waymark.db");
    let _env = EnvGuard::set(&[("WAYMARK_DB", db_path.as_path())]);

    let file = temp.path().join("valid.txt");
    fs::write(&file, "hello").unwrap();
    let fasd_path = temp.path().join("fasd");
    write_fixture(
        &fasd_path,
        &format!("{}|4.0|1710000000\nmalformed\n", file.display()),
    );

    let error = import_fasd(
        Some(&fasd_path),
        FasdImportOptions {
            strict: true,
            ..FasdImportOptions::default()
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("malformed fasd record"));
    assert!(!db_path.exists());
}

#[test]
fn missing_paths_are_skipped_by_default() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("waymark.db");
    let _env = EnvGuard::set(&[("WAYMARK_DB", db_path.as_path())]);

    let missing = temp.path().join("missing.txt");
    let fasd_path = temp.path().join("fasd");
    write_fixture(
        &fasd_path,
        &format!("{}|2.0|1710000000\n", missing.display()),
    );

    let summary = import_fasd(Some(&fasd_path), FasdImportOptions::default()).unwrap();

    assert_eq!(summary.parsed, 1);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 1);
    assert_eq!(summary.missing, 1);
    assert_eq!(summary.unknown, 0);
    assert!(!db_path.exists());
}

#[test]
fn keep_missing_stores_unknown_kind() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("waymark.db");
    let _env = EnvGuard::set(&[("WAYMARK_DB", db_path.as_path())]);

    let missing = temp.path().join("missing.txt");
    let fasd_path = temp.path().join("fasd");
    write_fixture(
        &fasd_path,
        &format!("{}|2.0|1710000000\n", missing.display()),
    );

    let summary = import_fasd(
        Some(&fasd_path),
        FasdImportOptions {
            keep_missing: true,
            ..FasdImportOptions::default()
        },
    )
    .unwrap();

    assert_eq!(summary.parsed, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.missing, 1);
    assert_eq!(summary.unknown, 1);

    let entry = fetch_entry(&db_path, &missing).unwrap();
    assert_eq!(entry.kind, "unknown");
    assert_float_eq(entry.rank, 2.0);
    assert_eq!(entry.last_accessed, 1710000000);
}

#[test]
fn dry_run_does_not_mutate_existing_database() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("waymark.db");
    let _env = EnvGuard::set(&[("WAYMARK_DB", db_path.as_path())]);

    let existing = temp.path().join("existing.txt");
    let new_file = temp.path().join("new.txt");
    fs::write(&existing, "old").unwrap();
    fs::write(&new_file, "new").unwrap();
    insert_entry(&db_path, &existing, "file", 7.0, 5, 100, 200, 200, "native");
    let fasd_path = temp.path().join("fasd");
    write_fixture(
        &fasd_path,
        &format!(
            "{}|3.0|300\n{}|4.0|400\n",
            existing.display(),
            new_file.display()
        ),
    );

    let summary = import_fasd(
        Some(&fasd_path),
        FasdImportOptions {
            dry_run: true,
            ..FasdImportOptions::default()
        },
    )
    .unwrap();

    assert_eq!(summary.parsed, 2);
    assert_eq!(summary.imported, 2);
    assert_eq!(summary.files, 2);

    let existing_entry = fetch_entry(&db_path, &existing).unwrap();
    assert_float_eq(existing_entry.rank, 7.0);
    assert_eq!(existing_entry.access_count, 5);
    assert_eq!(existing_entry.first_seen, 100);
    assert_eq!(existing_entry.last_accessed, 200);
    assert_eq!(existing_entry.last_seen, 200);
    assert_eq!(existing_entry.source, "native");
    assert!(fetch_entry(&db_path, &new_file).is_none());
}

#[test]
fn merge_combines_rank_and_uses_latest_timestamp() {
    let _lock = ENV_LOCK.lock().unwrap();
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("waymark.db");
    let _env = EnvGuard::set(&[("WAYMARK_DB", db_path.as_path())]);

    let file = temp.path().join("existing.txt");
    fs::write(&file, "old").unwrap();
    insert_entry(&db_path, &file, "file", 4.0, 7, 900, 1000, 1000, "native");
    let fasd_path = temp.path().join("fasd");
    write_fixture(&fasd_path, &format!("{}|2.5|1200\n", file.display()));

    let summary = import_fasd(Some(&fasd_path), FasdImportOptions::default()).unwrap();

    assert_eq!(summary.imported, 1);
    let entry = fetch_entry(&db_path, &file).unwrap();
    assert_eq!(entry.kind, "file");
    assert_float_eq(entry.rank, 6.5);
    assert_eq!(entry.access_count, 8);
    assert_eq!(entry.first_seen, 900);
    assert_eq!(entry.last_accessed, 1200);
    assert_eq!(entry.last_seen, 1200);
    assert_eq!(entry.source, "native");
}

struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, &Path)]) -> Self {
        Self::set_and_remove(vars, &[])
    }

    fn set_and_remove(vars: &[(&'static str, &Path)], remove: &[&'static str]) -> Self {
        let mut names = Vec::new();
        for &(name, _) in vars {
            names.push(name);
        }
        for &name in remove {
            if !names.contains(&name) {
                names.push(name);
            }
        }

        let saved = names
            .into_iter()
            .map(|name| (name, env::var_os(name)))
            .collect();

        for name in remove {
            unsafe {
                env::remove_var(name);
            }
        }
        for (name, value) in vars {
            unsafe {
                env::set_var(name, value);
            }
        }
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in self.saved.iter().rev() {
            unsafe {
                match value {
                    Some(value) => env::set_var(name, value),
                    None => env::remove_var(name),
                }
            }
        }
    }
}

#[derive(Debug)]
struct EntryRow {
    kind: String,
    rank: f64,
    access_count: i64,
    first_seen: i64,
    last_accessed: i64,
    last_seen: i64,
    source: String,
}

fn write_fixture(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn insert_entry(
    db_path: &Path,
    path: &Path,
    kind: &str,
    rank: f64,
    access_count: i64,
    first_seen: i64,
    last_accessed: i64,
    last_seen: i64,
    source: &str,
) {
    let connection = Connection::open(db_path).unwrap();
    create_schema(&connection);
    connection
        .execute(
            r#"
            insert into entries (
                path,
                kind,
                rank,
                access_count,
                first_seen,
                last_accessed,
                last_seen,
                source
            )
            values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                path.to_string_lossy().as_ref(),
                kind,
                rank,
                access_count,
                first_seen,
                last_accessed,
                last_seen,
                source
            ],
        )
        .unwrap();
}

fn fetch_entry(db_path: &Path, path: &Path) -> Option<EntryRow> {
    let connection = Connection::open(db_path).unwrap();
    connection
        .query_row(
            r#"
            select kind, rank, access_count, first_seen, last_accessed, last_seen, source
            from entries
            where path = ?1
            "#,
            params![path.to_string_lossy().as_ref()],
            |row| {
                Ok(EntryRow {
                    kind: row.get(0)?,
                    rank: row.get(1)?,
                    access_count: row.get(2)?,
                    first_seen: row.get(3)?,
                    last_accessed: row.get(4)?,
                    last_seen: row.get(5)?,
                    source: row.get(6)?,
                })
            },
        )
        .optional()
        .unwrap()
}

fn create_schema(connection: &Connection) {
    connection
        .execute_batch(
            r#"
            create table if not exists entries (
                path text primary key not null,
                kind text not null check (kind in ('file', 'dir', 'unknown')),
                rank real not null default 1.0,
                access_count integer not null default 1,
                first_seen integer not null,
                last_accessed integer not null,
                last_seen integer not null,
                source text not null default 'native'
            );
            "#,
        )
        .unwrap();
}

fn assert_float_eq(actual: f64, expected: f64) {
    let difference = (actual - expected).abs();
    assert!(
        difference < 0.000001,
        "expected {expected}, got {actual}, difference {difference}"
    );
}
