use crate::{
    checkpoint::{hex_hash, sync_parent, Cursor, Gap, SCHEMA_VERSION},
    config::{Config, Mode},
    failpoint,
};
use anyhow::{bail, Context, Result};
use log_proto::Record;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::{collections::BTreeMap, path::PathBuf};

pub struct SqliteSink {
    connection: Connection,
    path: PathBuf,
    confirmed: BTreeMap<String, Cursor>,
    pending: BTreeMap<String, Cursor>,
    dirty: bool,
    confirmed_gaps: u64,
    pending_gaps: u64,
    last_gap: Option<Gap>,
}
impl SqliteSink {
    pub fn open(
        config: &Config,
        initial: &BTreeMap<String, Cursor>,
        archive_id: &str,
        config_digest: &str,
    ) -> Result<Self> {
        let path = config
            .sqlite
            .as_ref()
            .context("missing sqlite config")?
            .path
            .clone();
        if config.mode == Mode::Create {
            // Claim the path without ever letting SQLite adopt an unrelated existing file.
            std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)?
                .sync_all()?;
            sync_parent(&path)?;
        }
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(std::time::Duration::from_secs(1))?;
        if config.mode == Mode::Resume {
            let integrity: String =
                connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
            if integrity != "ok" {
                bail!("SQLite integrity_check failed: {integrity}");
            }
            for (key, expected) in [
                ("schema_version", SCHEMA_VERSION.to_string()),
                ("archive_id", archive_id.into()),
                ("config_digest", config_digest.into()),
            ] {
                let actual: String = connection
                    .query_row("SELECT value FROM metadata WHERE key=?1", [key], |row| {
                        row.get(0)
                    })
                    .context("missing archive SQLite metadata")?;
                if actual != expected {
                    bail!("SQLite archive {key} mismatch");
                }
            }
        }
        let journal: String =
            connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        if journal.to_lowercase() != "wal" {
            bail!("SQLite WAL unavailable");
        }
        connection.pragma_update(None, "synchronous", "FULL")?;
        let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
        if synchronous != 2 {
            bail!("SQLite FULL synchronization unavailable");
        }
        if config.mode == Mode::Create {
            connection.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE records(stream TEXT NOT NULL,epoch TEXT NOT NULL,seq TEXT NOT NULL,key TEXT NOT NULL,payload BLOB NOT NULL,source_ts_ns TEXT,observed_ts_ns TEXT NOT NULL,upstream TEXT NOT NULL,upstream_epochs TEXT NOT NULL,channel TEXT,source_seq TEXT,record_sha256 TEXT NOT NULL,PRIMARY KEY(stream,epoch,seq));
CREATE TABLE checkpoints(stream TEXT PRIMARY KEY,epoch TEXT NOT NULL,next TEXT NOT NULL);
CREATE TABLE gaps(id INTEGER PRIMARY KEY,stream TEXT NOT NULL,epoch TEXT NOT NULL,first TEXT NOT NULL,last TEXT NOT NULL,reason TEXT NOT NULL,advances INTEGER NOT NULL,coverage_first TEXT NOT NULL);")?;
            for (key, value) in [
                ("schema_version", SCHEMA_VERSION.to_string()),
                ("archive_id", archive_id.into()),
                ("config_digest", config_digest.into()),
                ("initial", serde_json::to_string(initial)?),
            ] {
                connection.execute("INSERT INTO metadata VALUES(?1,?2)", params![key, value])?;
            }
            for (stream, cursor) in initial {
                connection.execute(
                    "INSERT INTO checkpoints VALUES(?1,?2,?3)",
                    params![stream, cursor.epoch, cursor.next.to_string()],
                )?;
            }
            connection.execute_batch("COMMIT")?;
            sync_parent(&path)?;
        }
        let mut confirmed = BTreeMap::new();
        {
            let mut statement = connection.prepare("SELECT stream,epoch,next FROM checkpoints")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let stream: String = row.get(0)?;
                let epoch: String = row.get(1)?;
                let next = unsigned(row.get(2)?)?;
                if epoch.is_empty() || next == 0 {
                    bail!("invalid SQLite checkpoint");
                }
                confirmed.insert(stream, Cursor { epoch, next });
            }
        }
        if confirmed.keys().collect::<std::collections::BTreeSet<_>>()
            != config.streams.iter().collect()
        {
            bail!("SQLite checkpoint streams mismatch");
        }
        let initial_json: String = connection.query_row(
            "SELECT value FROM metadata WHERE key='initial'",
            [],
            |row| row.get(0),
        )?;
        let stored_initial: BTreeMap<String, Cursor> = serde_json::from_str(&initial_json)?;
        for (stream, cursor) in &confirmed {
            let start = stored_initial
                .get(stream)
                .context("SQLite initial checkpoint missing")?;
            if start.epoch != cursor.epoch || start.next == 0 || start.next > cursor.next {
                bail!("SQLite checkpoint is inconsistent with initial cursor");
            }
        }
        validate_history(&connection, &stored_initial, &confirmed)?;
        let confirmed_gaps: u64 =
            connection.query_row("SELECT COUNT(*) FROM gaps", [], |row| row.get(0))?;
        Ok(Self {
            connection,
            path,
            pending: confirmed.clone(),
            confirmed,
            dirty: false,
            pending_gaps: confirmed_gaps,
            confirmed_gaps,
            last_gap: None,
        })
    }
    pub fn cursors(&self) -> BTreeMap<String, Cursor> {
        self.confirmed.clone()
    }
    fn begin(&mut self) -> Result<()> {
        if !self.dirty {
            self.connection.execute_batch("BEGIN IMMEDIATE")?;
            self.dirty = true;
        }
        Ok(())
    }
    pub fn accept(&mut self, record: &Record) -> Result<()> {
        let cursor = self
            .pending
            .get(&record.stream)
            .context("record for unconfigured stream")?;
        if cursor.epoch != record.epoch {
            bail!("epoch changed for {}", record.stream);
        }
        if record.seq < cursor.next {
            let existing = self.connection.query_row("SELECT key,payload,source_ts_ns,observed_ts_ns,upstream,upstream_epochs,channel,source_seq FROM records WHERE stream=?1 AND epoch=?2 AND seq=?3",params![record.stream,record.epoch,record.seq.to_string()],|row| {
                Ok((row.get::<_,String>(0)?,row.get::<_,Vec<u8>>(1)?,row.get::<_,Option<String>>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,Option<String>>(6)?,row.get::<_,Option<String>>(7)?))
            }).optional()?.context("duplicate identity absent from SQLite archive")?;
            let restored = Record {
                stream: record.stream.clone(),
                epoch: record.epoch.clone(),
                seq: record.seq,
                key: existing.0,
                payload: existing.1,
                source_ts_ns: existing.2.map(unsigned).transpose()?,
                observed_ts_ns: unsigned(existing.3)?,
                upstream: serde_json::from_str(&existing.4)?,
                upstream_epochs: serde_json::from_str(&existing.5)?,
                channel: existing.6,
                source_seq: existing.7.map(unsigned).transpose()?,
            };
            if restored != *record {
                bail!(
                    "conflicting duplicate Record {}/{}",
                    record.stream,
                    record.seq
                );
            }
            return Ok(());
        }
        if record.seq != cursor.next {
            bail!("unannounced gap in {}", record.stream);
        }
        let next = record
            .seq
            .checked_add(1)
            .context("record sequence exhausted u64 cursor space")?;
        self.begin()?;
        self.connection.execute(
            "INSERT INTO records VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                record.stream,
                record.epoch,
                record.seq.to_string(),
                record.key,
                record.payload,
                record.source_ts_ns.map(|v| v.to_string()),
                record.observed_ts_ns.to_string(),
                serde_json::to_string(&record.upstream)?,
                serde_json::to_string(&record.upstream_epochs)?,
                record.channel,
                record.source_seq.map(|value| value.to_string()),
                hex_hash(&serde_json::to_vec(record)?)
            ],
        )?;
        self.connection.execute(
            "UPDATE checkpoints SET next=?1 WHERE stream=?2",
            params![next.to_string(), record.stream],
        )?;
        self.pending.get_mut(&record.stream).unwrap().next = next;
        Ok(())
    }
    pub fn gap(&mut self, gap: &Gap, advances: bool) -> Result<()> {
        let cursor = self
            .pending
            .get(&gap.stream)
            .context("gap for unconfigured stream")?;
        if cursor.epoch != gap.epoch || gap.to < gap.from {
            bail!("invalid gap or epoch changed");
        }
        if gap.to < cursor.next && advances {
            return Ok(());
        }
        if gap.from > cursor.next {
            bail!("gap does not cover next expected record");
        }
        let coverage_first = cursor.next;
        let next = if advances {
            gap.to
                .checked_add(1)
                .context("gap sequence exhausted cursor space")?
                .max(cursor.next)
        } else {
            cursor.next
        };
        self.begin()?;
        self.connection.execute("INSERT INTO gaps(stream,epoch,first,last,reason,advances,coverage_first) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![gap.stream,gap.epoch,gap.from.to_string(),gap.to.to_string(),gap.reason,advances,coverage_first.to_string()])?;
        self.connection.execute(
            "UPDATE checkpoints SET next=?1 WHERE stream=?2",
            params![next.to_string(), gap.stream],
        )?;
        self.pending.get_mut(&gap.stream).unwrap().next = next;
        self.pending_gaps += 1;
        self.last_gap = Some(gap.clone());
        Ok(())
    }
    pub fn commit(&mut self) -> Result<()> {
        if self.dirty {
            failpoint("sqlite_before_commit");
            crate::errorpoint("sqlite_commit")?;
            self.connection
                .execute_batch("COMMIT")
                .context("SQLite commit failed; outcome may be unknown")?;
            self.confirmed = self.pending.clone();
            self.confirmed_gaps = self.pending_gaps;
            self.dirty = false;
            failpoint("sqlite_after_commit");
            sync_parent(&self.path)?;
        }
        Ok(())
    }
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({"confirmed":self.confirmed,"written":self.pending,"gap_count":self.confirmed_gaps,"incomplete":self.confirmed_gaps>0,"last_gap":self.last_gap,"journal_mode":"wal","synchronous":"full"})
    }
}
fn unsigned(value: String) -> Result<u64> {
    let parsed: u64 = value.parse().context("invalid SQLite unsigned decimal")?;
    if parsed.to_string() != value {
        bail!("noncanonical SQLite unsigned decimal");
    }
    Ok(parsed)
}

/// SQLite's integrity_check verifies pages, not archive completeness. Stream this
/// logical audit to detect deleted rows, edited content and checkpoint holes.
fn validate_history(
    connection: &Connection,
    initial: &BTreeMap<String, Cursor>,
    confirmed: &BTreeMap<String, Cursor>,
) -> Result<()> {
    let mut expected = initial.clone();
    let mut statement = connection.prepare("SELECT * FROM (
SELECT stream,epoch,seq AS first,seq AS last,0 AS kind,key,payload,source_ts_ns,observed_ts_ns,upstream,upstream_epochs,channel,source_seq,record_sha256 FROM records
UNION ALL
SELECT stream,epoch,coverage_first,last,1,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL FROM gaps WHERE advances=1
) ORDER BY stream,length(first),first,kind")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let stream: String = row.get(0)?;
        let epoch: String = row.get(1)?;
        let first = unsigned(row.get(2)?)?;
        let last = unsigned(row.get(3)?)?;
        let kind: i64 = row.get(4)?;
        let cursor = expected
            .get_mut(&stream)
            .context("archive history has an unknown stream")?;
        if epoch != cursor.epoch || first != cursor.next || last < first {
            bail!("SQLite archive history is discontinuous for {stream}");
        }
        if kind == 0 {
            let record = Record {
                stream: stream.clone(),
                epoch,
                seq: first,
                key: row.get(5)?,
                payload: row.get(6)?,
                source_ts_ns: row.get::<_, Option<String>>(7)?.map(unsigned).transpose()?,
                observed_ts_ns: unsigned(row.get(8)?)?,
                upstream: serde_json::from_str(&row.get::<_, String>(9)?)?,
                upstream_epochs: serde_json::from_str(&row.get::<_, String>(10)?)?,
                channel: row.get(11)?,
                source_seq: row
                    .get::<_, Option<String>>(12)?
                    .map(unsigned)
                    .transpose()?,
            };
            let expected_hash: String = row.get(13)?;
            if hex_hash(&serde_json::to_vec(&record)?) != expected_hash {
                bail!("SQLite Record content changed for {stream}/{first}");
            }
        }
        cursor.next = last
            .checked_add(1)
            .context("SQLite archive history overflows sequence space")?;
    }
    if &expected != confirmed {
        bail!("SQLite checkpoint disagrees with archive history");
    }
    let mut statement =
        connection.prepare("SELECT stream,epoch,first,last,advances,coverage_first FROM gaps")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let stream: String = row.get(0)?;
        let cursor = confirmed.get(&stream).context("gap has unknown stream")?;
        let epoch: String = row.get(1)?;
        let first = unsigned(row.get(2)?)?;
        let last = unsigned(row.get(3)?)?;
        let advances: i64 = row.get(4)?;
        let coverage = unsigned(row.get(5)?)?;
        if epoch != cursor.epoch
            || last < first
            || first > coverage
            || coverage > cursor.next
            || !(0..=1).contains(&advances)
        {
            bail!("SQLite gap metadata is invalid");
        }
    }
    Ok(())
}
