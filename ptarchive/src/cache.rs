use super::SESSION;
use crate::Arguments;

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use std::path::PathBuf;

const TARGET_SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Default)]
pub struct DigestRecord {
    pub digest: String,
    pub filename: String,
    pub absolute_path: String,
    pub length: u64,
    pub unique_id: Option<String>,
    pub session: String,
    pub hdd: String,
    pub repository: String,
    pub registry: String,
    pub tag: String,
}

pub fn init_pool(config: &Arguments) -> rusqlite::Result<r2d2::Pool<SqliteConnectionManager>> {
    if let Some(ref cache) = config.cache {
        let path = shellexpand::tilde(&cache.path);
        let manager = SqliteConnectionManager::file(path.as_ref());
        let pool = r2d2::Pool::new(manager).expect("couldn't create sqlite connection pool");

        // run our migration exactly once on pool creation
        let conn = pool.get().unwrap();
        conn.pragma_update(None, "journal_mode", &"WAL")?;
        conn.pragma_update(None, "synchronous", &"NORMAL")?;
        let current: i32 = conn.query_row("PRAGMA user_version", params![], |row| row.get(0))?;

        if current < TARGET_SCHEMA_VERSION {
            let sql = include_str!("../migrations/0001_create_digests.sql");
            conn.execute_batch(&sql)?;
            conn.pragma_update(None, "user_version", &TARGET_SCHEMA_VERSION)?;
        }

        Ok(pool)
    } else {
        Err(rusqlite::Error::InvalidPath(PathBuf::from("")))
    }
}

pub fn insert_record(conn: &rusqlite::Connection, rec: &DigestRecord) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT OR REPLACE INTO digests
         (digest, filename, absolute_path, length, unique_id, session, hdd, repository, registry, tag)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            rec.digest,
            rec.filename,
            rec.absolute_path,
            rec.length,
            rec.unique_id,
            rec.session,
            rec.hdd,
            rec.repository,
            rec.registry,
            rec.tag,
        ],
    )
}

pub fn get_digest_by_absolute_path(
    conn: &rusqlite::Connection,
    path: &str,
) -> rusqlite::Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT digest FROM digests WHERE absolute_path = ?1")?;
    let mut rows = stmt.query(params![path])?;

    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

pub fn get_digest_by_name_and_session(
    conn: &rusqlite::Connection,
    name: &str,
    //session: &str,
) -> rusqlite::Result<Option<String>> {
    let mut stmt =
        conn.prepare("SELECT digest FROM digests WHERE filename = ?1 and session = ?2")?;
    let mut rows = stmt.query(params![name, &SESSION.get().unwrap()])?;

    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

pub fn get_path_by_name() {
    unimplemented!()
}

pub fn get_record(
    conn: &rusqlite::Connection,
    digest: &str,
) -> rusqlite::Result<Option<DigestRecord>> {
    let mut stmt = conn.prepare(
        "SELECT digest, filename, absolute_path, session, hdd, repository, registry, tag
         FROM digests WHERE digest = ?1",
    )?;
    let mut rows = stmt.query(params![digest])?;

    if let Some(row) = rows.next()? {
        Ok(Some(DigestRecord {
            digest: row.get(0)?,
            filename: row.get(1)?,
            absolute_path: row.get(2)?,
            length: row.get(3)?,
            unique_id: row.get(4)?,
            session: row.get(5)?,
            hdd: row.get(6)?,
            repository: row.get(7)?,
            registry: row.get(8)?,
            tag: row.get(9)?,
        }))
    } else {
        Ok(None)
    }
}
