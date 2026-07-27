//! Database maintenance (vacuum/analyze).

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::Error;
use crate::log::AverLock;
use crate::store::ensure_sqlite_vec_registered;

/// Stats reported by `aver vacuum`. ADR-0019 §2.
#[derive(Debug, Clone)]
pub struct VacuumReport {
    pub pages_before: i64,
    pub freelist_before: i64,
    pub pages_after: i64,
    pub freelist_after: i64,
    pub vacuumed_into: Option<PathBuf>,
}

/// Run `VACUUM` (or `VACUUM INTO`) plus optional `ANALYZE` against an Aver
/// memory directory. Acquires the advisory lock for the duration. ADR-0019 §2.
pub fn vacuum(
    memory_dir: &Path,
    into: Option<&Path>,
    analyze: bool,
) -> Result<VacuumReport, Error> {
    // ADR-0017: VACUUM rewrites the whole database, including the `vec0`
    // virtual table (whose shadow tables exist as ordinary SQLite tables).
    // The extension must be loaded for SQLite to know about the module.
    ensure_sqlite_vec_registered();

    let _lock = AverLock::acquire(memory_dir)?;
    let db_path = memory_dir.join("db.sqlite");
    let conn = Connection::open(&db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let pages_before: i64 = conn.pragma_query_value(None, "page_count", |r| r.get(0))?;
    let freelist_before: i64 = conn.pragma_query_value(None, "freelist_count", |r| r.get(0))?;

    let vacuumed_into = if let Some(path) = into {
        // VACUUM INTO 'path' — does not block readers on origin.
        let path_str = path.to_string_lossy().replace('\'', "''");
        conn.execute_batch(&format!("VACUUM INTO '{path_str}'"))?;
        Some(path.to_path_buf())
    } else {
        conn.execute_batch("VACUUM")?;
        conn.execute_batch("PRAGMA optimize")?;
        None
    };
    if analyze {
        conn.execute_batch("ANALYZE")?;
    }

    let pages_after: i64 = conn.pragma_query_value(None, "page_count", |r| r.get(0))?;
    let freelist_after: i64 = conn.pragma_query_value(None, "freelist_count", |r| r.get(0))?;

    Ok(VacuumReport {
        pages_before,
        freelist_before,
        pages_after,
        freelist_after,
        vacuumed_into,
    })
}
