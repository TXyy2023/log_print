use anyhow::{bail, Context, Result};
use fs2::FileExt;
use log_proto::{Record, SaveOptions};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogEntry {
    index: usize,
    epoch: String,
    identity: String,
}
fn catalog_append(directory: &Path, index: usize, epoch: &str, identity: &str) -> Result<()> {
    let mut line = serde_json::to_vec(&CatalogEntry {
        index,
        epoch: epoch.into(),
        identity: identity.into(),
    })?;
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("catalog.jsonl"))?;
    file.write_all(&line)?;
    file.sync_all()?;
    #[cfg(unix)]
    File::open(directory)?.sync_all()?;
    Ok(())
}

/// Each stream has its own lock, transactions, limits, and immutable completed segments.
/// DELETE+EXTRA avoids an unbounded WAL. One segment-size journal reserve is charged.
pub struct Store {
    directory: PathBuf,
    files: Vec<PathBuf>,
    current: Connection,
    file_bytes: u64,
    total_bytes: u64,
    identity: String,
    pub epoch: String,
    pub head: u64,
    _lock: File,
}
fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn readonly(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}
fn configure(conn: &Connection, cap: u64) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=EXTRA; PRAGMA cache_size=-512; PRAGMA busy_timeout=1000;")?;
    conn.pragma_update(None, "max_page_count", cap / 4096)?;
    Ok(())
}
fn create(path: &Path, cap: u64, epoch: &str, identity: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn, cap)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS records(seq INTEGER PRIMARY KEY, key TEXT UNIQUE NOT NULL, json TEXT NOT NULL, checksum TEXT NOT NULL);")?;
    conn.execute("INSERT OR IGNORE INTO meta VALUES('epoch',?1)", [epoch])?;
    conn.execute(
        "INSERT OR IGNORE INTO meta VALUES('identity',?1)",
        [identity],
    )?;
    conn.execute("INSERT OR IGNORE INTO meta VALUES('head','0')", [])?;
    Ok(conn)
}
impl Store {
    pub fn open(id: &str, identity: &str, options: &SaveOptions) -> Result<Self> {
        let file_bytes = options.file_bytes.context("file_bytes not resolved")?;
        let total_bytes = options.total_bytes.context("total_bytes not resolved")?;
        if file_bytes < 262144 || total_bytes < file_bytes.saturating_mul(2) {
            bail!("save limits require file_bytes >= 262144 and total_bytes >= 2 * file_bytes (journal reserve)")
        }
        let directory =
            PathBuf::from(options.directory.as_ref().context("directory missing")?).join(id);
        fs::create_dir_all(&directory).context("create save directory")?;
        let mut lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(".lock"))?;
        lock.try_lock_exclusive()
            .context("stream storage is already locked by another Core")?;
        let mut files = fs::read_dir(&directory)?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        files.retain(|p| p.extension().is_some_and(|e| e == "sqlite"));
        files.sort();
        let catalog_path = directory.join("catalog.jsonl");
        let catalog = if catalog_path.exists() {
            if catalog_path.metadata()?.len() > 16 * 1024 * 1024 {
                bail!("history catalog exceeds supported size")
            }
            let bytes = fs::read(&catalog_path)?;
            if !bytes.ends_with(b"\n") {
                bail!("history catalog is incomplete; manual recovery required")
            }
            let entries = std::str::from_utf8(&bytes)?
                .lines()
                .map(serde_json::from_str::<CatalogEntry>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if entries.is_empty() || entries.len() != files.len() {
                bail!("history segment catalog mismatch: files may be missing")
            }
            entries
        } else {
            if !files.is_empty() || lock.metadata()?.len() > 0 {
                bail!("history catalog missing; refusing to reset a previously initialized stream")
            }
            Vec::new()
        };
        let mut epoch = uuid::Uuid::new_v4().to_string();
        let mut head = 0;
        for (i, path) in files.iter().enumerate() {
            if path.file_name().unwrap().to_string_lossy() != format!("{i:08}.sqlite") {
                bail!("history segment missing or renamed at index {i}")
            }
            // The last segment may have a hot rollback journal after process death.
            // Open existing storage read/write so SQLite can recover it; never CREATE
            // a missing file here, as the persistent catalog must detect that loss.
            let c = if i + 1 == files.len() {
                Connection::open_with_flags(
                    path,
                    OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?
            } else {
                readonly(path)?
            };
            let check: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
            if check != "ok" {
                bail!("history integrity failure: {check}")
            }
            let found: String =
                c.query_row("SELECT value FROM meta WHERE key='identity'", [], |r| {
                    r.get(0)
                })?;
            if found != identity {
                bail!("saved stream owner/parents changed; use a new stream or storage directory")
            }
            let e: String =
                c.query_row("SELECT value FROM meta WHERE key='epoch'", [], |r| r.get(0))?;
            if i == 0 {
                epoch = e
            } else if epoch != e {
                bail!("history epoch mismatch")
            }
            if catalog[i].index != i || catalog[i].epoch != epoch || catalog[i].identity != identity
            {
                bail!("history catalog identity mismatch")
            }
            let (min, max, count): (Option<u64>, Option<u64>, u64) =
                c.query_row("SELECT min(seq),max(seq),count(*) FROM records", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
            let committed_head: String =
                c.query_row("SELECT value FROM meta WHERE key='head'", [], |r| r.get(0))?;
            if committed_head.parse::<u64>()? != max.unwrap_or(0) {
                bail!("history committed head mismatch: records may have been deleted")
            }
            if let (Some(min), Some(max)) = (min, max) {
                if min != head + 1 || count != max - min + 1 {
                    bail!("history sequence gap at {min}")
                };
                head = max;
            }
        }
        if files.is_empty() {
            files.push(directory.join("00000000.sqlite"));
        }
        let current = create(files.last().unwrap(), file_bytes, &epoch, identity)?;
        if catalog.is_empty() {
            catalog_append(&directory, 0, &epoch, identity)?;
            lock.write_all(b"log-print/1 initialized\n")?;
            lock.sync_all()?;
        }
        let store = Self {
            directory,
            files,
            current,
            file_bytes,
            total_bytes,
            identity: identity.into(),
            epoch,
            head,
            _lock: lock,
        };
        if store.disk_bytes()?.saturating_add(file_bytes) > total_bytes {
            bail!("storage cap exceeded including journal reserve")
        }
        Ok(store)
    }
    pub fn disk_bytes(&self) -> Result<u64> {
        let mut n = 0;
        for entry in fs::read_dir(&self.directory)? {
            let m = entry?.metadata()?;
            if m.is_file() {
                n += m.len();
            }
        }
        Ok(n)
    }
    fn parse(json: String, checksum: String) -> Result<Record> {
        if digest(&json) != checksum {
            bail!("history record checksum mismatch")
        };
        Ok(serde_json::from_str(&json)?)
    }
    pub fn by_key(&self, key: &str) -> Result<Option<Record>> {
        for path in self.files.iter().rev() {
            let c = readonly(path)?;
            let found: Option<(String, String)> = c
                .query_row(
                    "SELECT json,checksum FROM records WHERE key=?1",
                    [key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((json, sum)) = found {
                return Ok(Some(Self::parse(json, sum)?));
            }
        }
        Ok(None)
    }
    pub fn append(&mut self, record: &Record) -> Result<()> {
        let json = serde_json::to_string(record)?;
        // Reserve enough SQLite pages for row, index splits and transaction metadata.
        let growth = (json.len() as u64 + 32768).div_ceil(4096) * 4096;
        if growth + 16384 > self.file_bytes {
            bail!("record does not fit configured save file_bytes")
        }
        let size: u64 = self
            .current
            .query_row("PRAGMA page_count", [], |r| r.get::<_, u64>(0))?
            * 4096;
        if size + growth > self.file_bytes {
            if self
                .disk_bytes()?
                .saturating_add(self.file_bytes)
                .saturating_add(growth + 16384)
                > self.total_bytes
            {
                bail!("storage total_bytes limit reached; history is retained")
            }
            let path = self
                .directory
                .join(format!("{:08}.sqlite", self.files.len()));
            self.current = create(&path, self.file_bytes, &self.epoch, &self.identity)?;
            catalog_append(
                &self.directory,
                self.files.len(),
                &self.epoch,
                &self.identity,
            )?;
            self.files.push(path);
        }
        if self
            .disk_bytes()?
            .saturating_add(growth)
            .saturating_add(self.file_bytes)
            > self.total_bytes
        {
            bail!("storage total_bytes limit reached including journal reserve")
        }
        let tx = self.current.transaction()?;
        tx.execute(
            "INSERT INTO records(seq,key,json,checksum) VALUES(?1,?2,?3,?4)",
            params![record.seq, record.key, json, digest(&json)],
        )?;
        tx.execute(
            "UPDATE meta SET value=?1 WHERE key='head'",
            [record.seq.to_string()],
        )?;
        tx.commit().context("commit_unknown: SQLite commit did not return success; retry same key only after manual resume")?;
        self.head = record.seq;
        Ok(())
    }
    pub fn read(&self, from: u64, limit: usize) -> Result<Vec<Record>> {
        let mut records = Vec::new();
        for path in &self.files {
            let c = readonly(path)?;
            let mut query =
                c.prepare("SELECT json,checksum FROM records WHERE seq>=?1 ORDER BY seq LIMIT ?2")?;
            let rows = query.query_map(params![from, limit - records.len()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (json, sum) = row?;
                records.push(Self::parse(json, sum)?);
            }
            if records.len() >= limit {
                break;
            }
        }
        Ok(records)
    }
}
