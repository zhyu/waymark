use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use crate::{Result, err};

#[derive(Clone, Debug, Default)]
pub struct FasdImportOptions {
    pub dry_run: bool,
    pub keep_missing: bool,
    pub strict: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FasdImportSummary {
    pub parsed: usize,
    pub imported: usize,
    pub skipped: usize,
    pub malformed: usize,
    pub missing: usize,
    pub files: usize,
    pub dirs: usize,
    pub unknown: usize,
}

#[derive(Clone, Debug)]
struct FasdRecord {
    path: PathBuf,
    rank: f64,
    timestamp: i64,
}

#[derive(Clone, Debug)]
struct ImportRecord {
    path: PathBuf,
    kind: ImportKind,
    rank: f64,
    timestamp: i64,
}

#[derive(Clone, Debug)]
struct ClassifiedRecord {
    record: ImportRecord,
    missing: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImportKind {
    File,
    Dir,
    Unknown,
}

impl ImportKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "dir",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug)]
struct MalformedRecord {
    line_number: usize,
    reason: String,
}

pub fn import_fasd(from: Option<&Path>, options: FasdImportOptions) -> Result<FasdImportSummary> {
    let source = resolve_source(from)?;
    let input = fs::read_to_string(&source)?;
    let mut summary = FasdImportSummary::default();
    let mut import_records = Vec::new();
    let mut malformed_records = Vec::new();

    for (line_index, raw_line) in input.lines().enumerate() {
        let line_number = line_index + 1;
        match parse_record(raw_line, line_number) {
            Ok(record) => {
                summary.parsed += 1;
                match classify_record(record, options.keep_missing)? {
                    Some(classified) => {
                        if classified.missing {
                            summary.missing += 1;
                        }
                        increment_kind_counts(&mut summary, classified.record.kind);
                        summary.imported += 1;
                        import_records.push(classified.record);
                    }
                    None => {
                        summary.missing += 1;
                        summary.skipped += 1;
                    }
                }
            }
            Err(malformed) => {
                summary.malformed += 1;
                summary.skipped += 1;
                malformed_records.push(malformed);
            }
        }
    }

    if options.strict && !malformed_records.is_empty() {
        let first = &malformed_records[0];
        return Err(err(format!(
            "malformed fasd record at {}:{}: {}",
            source.display(),
            first.line_number,
            first.reason
        )));
    }

    if options.dry_run || import_records.is_empty() {
        return Ok(summary);
    }

    write_import_records(&import_records)?;
    Ok(summary)
}

fn parse_record(
    raw_line: &str,
    line_number: usize,
) -> std::result::Result<FasdRecord, MalformedRecord> {
    let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
    let mut parts = line.rsplitn(3, '|');
    let timestamp = parts
        .next()
        .ok_or_else(|| malformed(line_number, "missing timestamp"))?;
    let rank = parts
        .next()
        .ok_or_else(|| malformed(line_number, "missing rank"))?;
    let path = parts
        .next()
        .ok_or_else(|| malformed(line_number, "missing path"))?;

    if path.is_empty() {
        return Err(malformed(line_number, "empty path"));
    }

    let rank = rank
        .parse::<f64>()
        .map_err(|_| malformed(line_number, "invalid rank"))?;
    if !rank.is_finite() {
        return Err(malformed(line_number, "invalid rank"));
    }

    let timestamp = timestamp
        .parse::<i64>()
        .map_err(|_| malformed(line_number, "invalid timestamp"))?;

    Ok(FasdRecord {
        path: PathBuf::from(path),
        rank,
        timestamp,
    })
}

fn malformed(line_number: usize, reason: impl Into<String>) -> MalformedRecord {
    MalformedRecord {
        line_number,
        reason: reason.into(),
    }
}

fn classify_record(record: FasdRecord, keep_missing: bool) -> Result<Option<ClassifiedRecord>> {
    match fs::metadata(&record.path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(ClassifiedRecord {
            record: ImportRecord {
                path: record.path,
                kind: ImportKind::File,
                rank: record.rank,
                timestamp: record.timestamp,
            },
            missing: false,
        })),
        Ok(metadata) if metadata.is_dir() => Ok(Some(ClassifiedRecord {
            record: ImportRecord {
                path: record.path,
                kind: ImportKind::Dir,
                rank: record.rank,
                timestamp: record.timestamp,
            },
            missing: false,
        })),
        Ok(_) if keep_missing => Ok(Some(ClassifiedRecord {
            record: ImportRecord {
                path: record.path,
                kind: ImportKind::Unknown,
                rank: record.rank,
                timestamp: record.timestamp,
            },
            missing: true,
        })),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if keep_missing {
                Ok(Some(ClassifiedRecord {
                    record: ImportRecord {
                        path: record.path,
                        kind: ImportKind::Unknown,
                        rank: record.rank,
                        timestamp: record.timestamp,
                    },
                    missing: true,
                }))
            } else {
                Ok(None)
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn increment_kind_counts(summary: &mut FasdImportSummary, kind: ImportKind) {
    match kind {
        ImportKind::File => summary.files += 1,
        ImportKind::Dir => summary.dirs += 1,
        ImportKind::Unknown => summary.unknown += 1,
    }
}

fn write_import_records(records: &[ImportRecord]) -> Result<()> {
    let db_path = database_path()?;
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut connection = Connection::open(db_path)?;
    ensure_schema(&connection)?;
    let transaction = connection.transaction()?;

    {
        let mut statement = transaction.prepare(
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
            values (?1, ?2, ?3, 1, ?4, ?4, ?4, 'fasd')
            on conflict(path) do update set
                kind = case
                    when entries.kind = 'unknown' then excluded.kind
                    else entries.kind
                end,
                rank = entries.rank + excluded.rank,
                access_count = entries.access_count + excluded.access_count,
                first_seen = min(entries.first_seen, excluded.first_seen),
                last_accessed = max(entries.last_accessed, excluded.last_accessed),
                last_seen = max(entries.last_seen, excluded.last_seen)
            "#,
        )?;

        for record in records {
            let path = record.path.to_string_lossy();
            statement.execute(params![
                path.as_ref(),
                record.kind.as_str(),
                record.rank,
                record.timestamp
            ])?;
        }
    }

    transaction.commit()?;
    Ok(())
}

fn ensure_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
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

        create index if not exists entries_kind_last_accessed
          on entries(kind, last_accessed);

        create index if not exists entries_kind_rank
          on entries(kind, rank);
        "#,
    )?;
    Ok(())
}

fn resolve_source(from: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = from {
        return Ok(path.to_path_buf());
    }

    for path in default_fasd_paths() {
        if path.is_file() {
            return Ok(path);
        }
    }

    Err(err("no fasd data file found in default locations"))
}

fn default_fasd_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(xdg_cache_home) = non_empty_env_path("XDG_CACHE_HOME") {
        paths.push(xdg_cache_home.join("fasd"));
    }

    if let Some(home) = non_empty_env_path("HOME") {
        paths.push(home.join(".cache").join("fasd"));
        paths.push(home.join(".fasd"));
    }

    paths
}

fn database_path() -> Result<PathBuf> {
    if let Some(path) = non_empty_env_path("WAYMARK_DB") {
        return Ok(path);
    }

    let base = if let Some(xdg_data_home) = non_empty_env_path("XDG_DATA_HOME") {
        xdg_data_home
    } else {
        let home = non_empty_env_path("HOME")
            .ok_or_else(|| err("HOME must be set when WAYMARK_DB and XDG_DATA_HOME are unset"))?;
        home.join(".local").join("share")
    };

    Ok(base.join("waymark").join("waymark.db"))
}

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).and_then(|value| {
        if value.is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    })
}
