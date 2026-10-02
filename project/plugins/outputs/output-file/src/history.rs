//! Independent, read-only access to committed SQLite archives (schemas 2 and 3).
//! A watermark pins each query; no connection or lock is shared with the writer.
use crate::{Cursor, Gap};
use anyhow::{bail, Context, Result};
use log_proto::Record;
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistorySnapshot {
    pub archive_id: String,
    pub schema_version: u32,
    pub initial: BTreeMap<String, Cursor>,
    pub committed: BTreeMap<String, Cursor>,
    pub gaps: Vec<Gap>,
    pub gap_count: usize,
}
pub struct HistoryReader {
    connection: Connection,
}
impl HistoryReader {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(std::time::Duration::from_millis(1000))?;
        connection.pragma_update(None, "query_only", true)?;
        let reader = Self { connection };
        reader.snapshot()?;
        Ok(reader)
    }
    pub fn snapshot(&self) -> Result<HistorySnapshot> {
        let tx = self.connection.unchecked_transaction()?;
        let metadata = |key: &str| -> Result<String> {
            Ok(
                tx.query_row("SELECT value FROM metadata WHERE key=?1", [key], |r| {
                    r.get(0)
                })?,
            )
        };
        let schema_version: u32 = metadata("schema_version")?.parse()?;
        if ![2, 3].contains(&schema_version) {
            bail!("unsupported SQLite archive schema {schema_version}");
        }
        let archive_id = metadata("archive_id")?;
        uuid::Uuid::parse_str(&archive_id).context("invalid archive identity")?;
        let initial: BTreeMap<String, Cursor> = serde_json::from_str(&metadata("initial")?)?;
        let mut committed = BTreeMap::new();
        let mut statement = tx.prepare("SELECT stream,epoch,next FROM checkpoints")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let stream: String = row.get(0)?;
            let cursor = Cursor {
                epoch: row.get(1)?,
                next: row.get::<_, String>(2)?.parse()?,
            };
            let first = initial.get(&stream).context("missing initial coverage")?;
            if cursor.epoch != first.epoch || first.next == 0 || cursor.next < first.next {
                bail!("invalid committed coverage");
            }
            committed.insert(stream, cursor);
        }
        drop(rows);
        drop(statement);
        let gap_count: usize = tx.query_row("SELECT COUNT(*) FROM gaps", [], |r| r.get(0))?;
        let mut gaps = Vec::new();
        let mut statement =
            tx.prepare("SELECT stream,epoch,first,last,reason FROM gaps ORDER BY id LIMIT 200")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            gaps.push(Gap {
                stream: row.get(0)?,
                epoch: row.get(1)?,
                from: row.get::<_, String>(2)?.parse()?,
                to: row.get::<_, String>(3)?.parse()?,
                reason: row.get(4)?,
            });
        }
        drop(rows);
        drop(statement);
        tx.commit()?;
        Ok(HistorySnapshot {
            archive_id,
            schema_version,
            initial,
            committed,
            gaps,
            gap_count,
        })
    }
    /// Results are in numeric sequence order, including the full u64 range.
    /// Never reads beyond the caller's fixed committed watermark.
    pub fn read(
        &self,
        snapshot: &HistorySnapshot,
        stream: &str,
        epoch: &str,
        from: u64,
        end: u64,
        limit: usize,
    ) -> Result<Vec<Record>> {
        if limit == 0 || limit > 512 {
            bail!("history batch limit must be 1..512");
        }
        let current_id: String = self.connection.query_row(
            "SELECT value FROM metadata WHERE key='archive_id'",
            [],
            |r| r.get(0),
        )?;
        if current_id != snapshot.archive_id {
            bail!("archive identity changed");
        }
        let cursor = snapshot
            .committed
            .get(stream)
            .context("stream is not covered by archive")?;
        if cursor.epoch != epoch {
            bail!("archive epoch does not match current runtime");
        }
        let end = end.min(cursor.next.saturating_sub(1));
        if end < from {
            return Ok(Vec::new());
        }
        let first = from.to_string();
        let last = end.to_string();
        let mut statement = self.connection.prepare("SELECT stream,epoch,seq,key,payload,source_ts_ns,observed_ts_ns,upstream,upstream_epochs,channel,source_seq FROM records WHERE stream=?1 AND epoch=?2 AND (length(seq)>length(?3) OR (length(seq)=length(?3) AND seq>=?3)) AND (length(seq)<length(?4) OR (length(seq)=length(?4) AND seq<=?4)) ORDER BY length(seq),seq LIMIT ?5")?;
        let mut rows = statement.query(params![stream, epoch, first, last, limit])?;
        let mut records = Vec::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let record = Record {
                stream: row.get(0)?,
                epoch: row.get(1)?,
                seq: row.get::<_, String>(2)?.parse()?,
                key: row.get(3)?,
                payload: row.get(4)?,
                source_ts_ns: row
                    .get::<_, Option<String>>(5)?
                    .map(|s| s.parse())
                    .transpose()?,
                observed_ts_ns: row.get::<_, String>(6)?.parse()?,
                upstream: serde_json::from_str(&row.get::<_, String>(7)?)?,
                upstream_epochs: serde_json::from_str(&row.get::<_, String>(8)?)?,
                channel: row.get(9)?,
                source_seq: row
                    .get::<_, Option<String>>(10)?
                    .map(|s| s.parse())
                    .transpose()?,
            };
            bytes += record.payload.len();
            if bytes > 4 * 1024 * 1024 && !records.is_empty() {
                break;
            }
            records.push(record);
        }
        Ok(records)
    }
}
