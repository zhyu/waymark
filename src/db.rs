use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::ranking;
use crate::{Result, err};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    File,
    Dir,
    Any,
    Auto,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct EntryInput {
    pub path: PathBuf,
    pub kind: Kind,
    pub rank: f64,
    pub access_count: i64,
    pub first_seen: i64,
    pub last_accessed: i64,
    pub last_seen: i64,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub path: String,
    pub kind: String,
    pub rank: f64,
    pub access_count: i64,
    pub first_seen: i64,
    pub last_accessed: i64,
    pub last_seen: i64,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScoredEntry {
    pub entry: Entry,
    pub score: f64,
}

pub struct Database {
    conn: Connection,
    path: PathBuf,
}

impl Database {
    pub fn open_default() -> Result<Self> {
        Self::open(default_db_path()?)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&path)?;
        conn.busy_timeout(std::time::Duration::from_millis(1_000))?;
        conn.execute_batch(
            "
            pragma foreign_keys = on;
            pragma journal_mode = wal;

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

            create index if not exists entries_kind_last_accessed
              on entries(kind, last_accessed);

            create index if not exists entries_kind_rank
              on entries(kind, rank);
            ",
        )?;

        Ok(Self { conn, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn add_paths(&self, requested_kind: Kind, paths: &[PathBuf]) -> Result<()> {
        let now = unix_timestamp()?;
        for path in paths {
            let normalized = normalize_absolute_lexical(path)?;
            let Some(actual_kind) = classify_existing_path(&normalized) else {
                continue;
            };
            let Some(kind) = requested_kind_for_existing(requested_kind, actual_kind) else {
                continue;
            };
            self.record_native_path(&normalized, kind, now)?;
        }
        Ok(())
    }

    pub fn add_entries(&self, entries: &[EntryInput]) -> Result<()> {
        for entry in entries {
            self.merge_entry(entry)?;
        }
        Ok(())
    }

    pub fn query(&self, kind: Kind, tokens: &[String], limit: usize) -> Result<Vec<ScoredEntry>> {
        let entries = self.load_entries(kind)?;
        let mut results = ranking::score_entries(entries, tokens, unix_timestamp()?);
        results.truncate(limit);
        Ok(results)
    }

    pub fn all_entries(&self) -> Result<Vec<Entry>> {
        self.load_entries(Kind::Any)
    }

    pub fn delete_paths(&self, paths: &[PathBuf]) -> Result<usize> {
        let mut deleted = 0;
        for path in paths {
            let normalized = normalize_absolute_lexical(path)?;
            deleted += self.conn.execute(
                "delete from entries where path = ?1",
                params![path_to_text(&normalized)],
            )?;
        }
        Ok(deleted)
    }

    pub fn prune_missing(&self) -> Result<usize> {
        let entries = self.load_entries(Kind::Any)?;
        let mut pruned = 0;
        for entry in entries {
            if !Path::new(&entry.path).exists() {
                pruned += self
                    .conn
                    .execute("delete from entries where path = ?1", params![entry.path])?;
            }
        }
        Ok(pruned)
    }

    pub fn doctor() -> Result<String> {
        let db = Self::open_default()?;
        db.doctor_report()
    }

    fn record_native_path(&self, path: &Path, kind: Kind, now: i64) -> Result<()> {
        let path = path_to_text(path);
        let kind = kind
            .storage_name()
            .ok_or_else(|| err(format!("cannot store non-entry kind {kind:?}")))?;

        self.conn.execute(
            "
            insert into entries (
              path, kind, rank, access_count, first_seen, last_accessed, last_seen, source
            )
            values (?1, ?2, 1.0, 1, ?3, ?3, ?3, 'native')
            on conflict(path) do update set
              kind = excluded.kind,
              rank = entries.rank + (1.0 / case when entries.rank <= 0.0 then 1.0 else entries.rank end),
              access_count = entries.access_count + 1,
              last_accessed = excluded.last_accessed,
              last_seen = excluded.last_seen
            ",
            params![path, kind, now],
        )?;
        Ok(())
    }

    fn merge_entry(&self, input: &EntryInput) -> Result<()> {
        let path = normalize_absolute_lexical(&input.path)?;
        let kind = input
            .kind
            .storage_name()
            .ok_or_else(|| err(format!("cannot store non-entry kind {:?}", input.kind)))?;
        let path = path_to_text(&path);
        let rank = input.rank.max(1.0);
        let access_count = input.access_count.max(1);

        self.conn.execute(
            "
            insert into entries (
              path, kind, rank, access_count, first_seen, last_accessed, last_seen, source
            )
            values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            on conflict(path) do update set
              kind = excluded.kind,
              rank = entries.rank + excluded.rank,
              access_count = entries.access_count + excluded.access_count,
              first_seen = min(entries.first_seen, excluded.first_seen),
              last_accessed = max(entries.last_accessed, excluded.last_accessed),
              last_seen = max(entries.last_seen, excluded.last_seen),
              source = excluded.source
            ",
            params![
                path,
                kind,
                rank,
                access_count,
                input.first_seen,
                input.last_accessed,
                input.last_seen,
                input.source
            ],
        )?;
        Ok(())
    }

    fn load_entries(&self, kind: Kind) -> Result<Vec<Entry>> {
        let select = "
            select path, kind, rank, access_count, first_seen, last_accessed, last_seen, source
            from entries
        ";
        let mut entries = Vec::new();

        match kind.query_storage_name() {
            Some(kind) => {
                let mut stmt = self.conn.prepare(&format!("{select} where kind = ?1"))?;
                let rows = stmt.query_map(params![kind], row_to_entry)?;
                for row in rows {
                    entries.push(row?);
                }
            }
            None => {
                let mut stmt = self.conn.prepare(select)?;
                let rows = stmt.query_map([], row_to_entry)?;
                for row in rows {
                    entries.push(row?);
                }
            }
        }

        Ok(entries)
    }

    fn doctor_report(&self) -> Result<String> {
        let mut counts = Counts::default();
        let mut stmt = self
            .conn
            .prepare("select kind, count(*) from entries group by kind")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (kind, count) = row?;
            match kind.as_str() {
                "file" => counts.files = count,
                "dir" => counts.dirs = count,
                "unknown" => counts.unknown = count,
                _ => {}
            }
        }

        let mut missing = 0_i64;
        let mut stmt = self.conn.prepare("select path from entries")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            if !Path::new(&row?).exists() {
                missing += 1;
            }
        }

        let total = counts.files + counts.dirs + counts.unknown;
        Ok(format!(
            "database_path={}\nentries_total={total}\nentries_file={}\nentries_dir={}\nentries_unknown={}\nentries_missing={missing}\n",
            self.path.display(),
            counts.files,
            counts.dirs,
            counts.unknown
        ))
    }
}

impl Kind {
    fn storage_name(self) -> Option<&'static str> {
        match self {
            Self::File => Some("file"),
            Self::Dir => Some("dir"),
            Self::Unknown => Some("unknown"),
            Self::Any | Self::Auto => None,
        }
    }

    fn query_storage_name(self) -> Option<&'static str> {
        match self {
            Self::File => Some("file"),
            Self::Dir => Some("dir"),
            Self::Unknown => Some("unknown"),
            Self::Any | Self::Auto => None,
        }
    }
}

#[derive(Default)]
struct Counts {
    files: i64,
    dirs: i64,
    unknown: i64,
}

fn default_db_path() -> Result<PathBuf> {
    default_db_path_from_env(|key| std::env::var_os(key))
}

fn default_db_path_from_env(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf> {
    if let Some(path) = nonempty_var(&var, "WAYMARK_DB") {
        return Ok(PathBuf::from(path));
    }

    if let Some(data_home) = nonempty_var(&var, "XDG_DATA_HOME") {
        return Ok(PathBuf::from(data_home).join("waymark").join("waymark.db"));
    }

    if let Some(home) = nonempty_var(&var, "HOME") {
        return Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("waymark")
            .join("waymark.db"));
    }

    Err(err(
        "cannot determine database path: set WAYMARK_DB or HOME",
    ))
}

fn nonempty_var(var: &impl Fn(&str) -> Option<OsString>, key: &str) -> Option<OsString> {
    var(key).filter(|value| !value.is_empty())
}

fn normalize_absolute_lexical(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let at_root = normalized
                    .components()
                    .next_back()
                    .is_some_and(|component| matches!(component, Component::RootDir));
                if !at_root {
                    normalized.pop();
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    Ok(normalized)
}

fn classify_existing_path(path: &Path) -> Option<Kind> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.is_file() {
        Some(Kind::File)
    } else if metadata.is_dir() {
        Some(Kind::Dir)
    } else {
        None
    }
}

fn requested_kind_for_existing(requested: Kind, actual: Kind) -> Option<Kind> {
    match requested {
        Kind::Auto | Kind::Any => Some(actual),
        Kind::File if actual == Kind::File => Some(Kind::File),
        Kind::Dir if actual == Kind::Dir => Some(Kind::Dir),
        Kind::Unknown => Some(Kind::Unknown),
        _ => None,
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        path: row.get(0)?,
        kind: row.get(1)?,
        rank: row.get(2)?,
        access_count: row.get(3)?,
        first_seen: row.get(4)?,
        last_accessed: row.get(5)?,
        last_seen: row.get(6)?,
        source: row.get(7)?,
    })
}

fn unix_timestamp() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| err(format!("system clock is before unix epoch: {error}")))?
        .as_secs() as i64)
}

fn path_to_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn default_path_prefers_waymark_db_then_xdg_then_home() {
        let mut vars = HashMap::new();
        vars.insert("WAYMARK_DB", OsString::from("/tmp/custom.db"));
        vars.insert("XDG_DATA_HOME", OsString::from("/tmp/xdg"));
        vars.insert("HOME", OsString::from("/tmp/home"));
        assert_eq!(
            default_db_path_from_env(|key| vars.get(key).cloned()).unwrap(),
            PathBuf::from("/tmp/custom.db")
        );

        vars.remove("WAYMARK_DB");
        assert_eq!(
            default_db_path_from_env(|key| vars.get(key).cloned()).unwrap(),
            PathBuf::from("/tmp/xdg/waymark/waymark.db")
        );

        vars.remove("XDG_DATA_HOME");
        assert_eq!(
            default_db_path_from_env(|key| vars.get(key).cloned()).unwrap(),
            PathBuf::from("/tmp/home/.local/share/waymark/waymark.db")
        );
    }

    #[test]
    fn add_paths_normalizes_and_skips_missing_paths() {
        let temp = tempdir().unwrap();
        let db = Database::open(temp.path().join("waymark.db")).unwrap();
        let dir = temp.path().join("nested");
        fs::create_dir(&dir).unwrap();
        let file = dir.join("file.txt");
        fs::write(&file, "content").unwrap();

        db.add_paths(
            Kind::File,
            &[
                temp.path().join("nested/../nested/./file.txt"),
                temp.path().join("missing.txt"),
            ],
        )
        .unwrap();

        let results = db.query(Kind::File, &[], 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].entry.path,
            temp.path().join("nested/file.txt").display().to_string()
        );
    }

    #[test]
    fn add_auto_classifies_files_and_dirs_and_query_filters() {
        let temp = tempdir().unwrap();
        let db = Database::open(temp.path().join("waymark.db")).unwrap();
        let dir = temp.path().join("project");
        fs::create_dir(&dir).unwrap();
        let file = temp.path().join("README.md");
        fs::write(&file, "content").unwrap();

        db.add_paths(Kind::Auto, &[dir.clone(), file.clone()])
            .unwrap();

        let files = db.query(Kind::File, &[], 20).unwrap();
        let dirs = db.query(Kind::Dir, &[], 20).unwrap();
        let any = db.query(Kind::Any, &[], 20).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].entry.kind, "file");
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].entry.kind, "dir");
        assert_eq!(any.len(), 2);
    }

    #[test]
    fn repeated_add_updates_frequency_fields() {
        let temp = tempdir().unwrap();
        let db = Database::open(temp.path().join("waymark.db")).unwrap();
        let file = temp.path().join("README.md");
        fs::write(&file, "content").unwrap();

        db.add_paths(Kind::File, std::slice::from_ref(&file))
            .unwrap();
        db.add_paths(Kind::File, std::slice::from_ref(&file))
            .unwrap();

        let results = db.query(Kind::File, &["readme".to_string()], 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.access_count, 2);
        assert!(results[0].entry.rank > 1.0);
    }

    #[test]
    fn doctor_reports_path_counts_and_missing_records() {
        let temp = tempdir().unwrap();
        let db = Database::open(temp.path().join("waymark.db")).unwrap();
        let file = temp.path().join("tracked.txt");
        fs::write(&file, "content").unwrap();
        let missing = temp.path().join("missing.txt");

        db.add_paths(Kind::File, std::slice::from_ref(&file))
            .unwrap();
        db.add_entries(&[EntryInput {
            path: missing,
            kind: Kind::Unknown,
            rank: 1.0,
            access_count: 1,
            first_seen: 10,
            last_accessed: 10,
            last_seen: 10,
            source: "test".to_string(),
        }])
        .unwrap();

        let report = db.doctor_report().unwrap();
        assert!(report.contains(&format!("database_path={}", db.path().display())));
        assert!(report.contains("entries_total=2"));
        assert!(report.contains("entries_file=1"));
        assert!(report.contains("entries_unknown=1"));
        assert!(report.contains("entries_missing=1"));
    }
}
