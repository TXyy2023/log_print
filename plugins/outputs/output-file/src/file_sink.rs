use crate::{
    checkpoint::*,
    config::{Config, Format, Mode},
    failpoint,
};
use anyhow::{bail, Context, Result};
use log_proto::Record;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum IndexEntry {
    Record {
        epoch: String,
        seq: u64,
        record_sha256: String,
        offset: u64,
        length: u64,
    },
    Gap {
        gap: Gap,
        advances: bool,
    },
}
struct StreamFile {
    path: PathBuf,
    data: File,
    index: File,
    confirmed: FileCheckpoint,
    pending: FileCheckpoint,
    data_hash: Sha256,
    index_hash: Sha256,
    last_gap: Option<Gap>,
}
pub struct FileSink {
    format: Format,
    streams: BTreeMap<String, StreamFile>,
}
impl FileSink {
    pub fn open(
        config: &Config,
        initial: &BTreeMap<String, Cursor>,
        archive_id: &str,
        config_digest: &str,
    ) -> Result<Self> {
        let file = config.file.as_ref().context("missing file config")?;
        let mut streams = BTreeMap::new();
        for (stream, path) in &file.paths {
            let checkpoint_path = sibling(path, ".checkpoint.json");
            let index_path = sibling(path, ".records.jsonl");
            let (mut data, mut index, checkpoint) = if config.mode == Mode::Create {
                let data = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                let index = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&index_path)?;
                data.sync_all()?;
                index.sync_all()?;
                sync_parent(path)?;
                let cursor = initial
                    .get(stream)
                    .context("missing initial stream cursor")?
                    .clone();
                let cp = FileCheckpoint {
                    schema_version: SCHEMA_VERSION,
                    archive_id: archive_id.into(),
                    config_digest: config_digest.into(),
                    stream: stream.clone(),
                    initial: cursor.clone(),
                    cursor,
                    file_identity: identity(&data)?,
                    index_identity: identity(&index)?,
                    confirmed_len: 0,
                    confirmed_sha256: hex_hash(b""),
                    index_len: 0,
                    index_sha256: hex_hash(b""),
                    gap_count: 0,
                    incomplete: false,
                };
                save_checkpoint(&checkpoint_path, &cp)?;
                failpoint("init_after_first_target");
                (data, index, cp)
            } else {
                let cp: FileCheckpoint = serde_json::from_reader(
                    File::open(&checkpoint_path)
                        .context("missing file checkpoint; initialization may be incomplete")?,
                )?;
                if cp.schema_version != SCHEMA_VERSION
                    || cp.archive_id != archive_id
                    || cp.config_digest != config_digest
                    || cp.stream != *stream
                    || cp.cursor.epoch != cp.initial.epoch
                    || cp.cursor.next < cp.initial.next
                    || cp.initial.next == 0
                {
                    bail!("file checkpoint identity/schema/config/cursor mismatch for {stream}");
                }
                let data = OpenOptions::new().read(true).write(true).open(path)?;
                let index = OpenOptions::new().read(true).write(true).open(index_path)?;
                if identity(&data)? != cp.file_identity || identity(&index)? != cp.index_identity {
                    bail!("archive file identity changed for {stream}");
                }
                (data, index, cp)
            };
            let data_hash = prefix_hash(&mut data, checkpoint.confirmed_len)?;
            let index_hash = prefix_hash(&mut index, checkpoint.index_len)?;
            if digest(&data_hash) != checkpoint.confirmed_sha256
                || digest(&index_hash) != checkpoint.index_sha256
            {
                bail!("archive confirmed content changed for {stream}");
            }
            validate_index(&mut index, &checkpoint)?;
            // Both identities and both confirmed prefixes have passed before any truncation.
            data.set_len(checkpoint.confirmed_len)?;
            index.set_len(checkpoint.index_len)?;
            data.sync_all()?;
            index.sync_all()?;
            data.seek(SeekFrom::End(0))?;
            index.seek(SeekFrom::End(0))?;
            streams.insert(
                stream.clone(),
                StreamFile {
                    path: path.clone(),
                    data,
                    index,
                    confirmed: checkpoint.clone(),
                    pending: checkpoint,
                    data_hash,
                    index_hash,
                    last_gap: None,
                },
            );
        }
        Ok(Self {
            format: file.format,
            streams,
        })
    }
    pub fn cursors(&self) -> BTreeMap<String, Cursor> {
        self.streams
            .iter()
            .map(|(s, f)| (s.clone(), f.confirmed.cursor.clone()))
            .collect()
    }
    pub fn accept(&mut self, record: &Record) -> Result<()> {
        let f = self
            .streams
            .get_mut(&record.stream)
            .context("record for unconfigured stream")?;
        if record.epoch != f.pending.cursor.epoch {
            bail!("epoch changed for {}", record.stream);
        }
        let record_hash = hex_hash(&serde_json::to_vec(record)?);
        if record.seq < f.pending.cursor.next {
            // The on-disk index keeps memory bounded even for arbitrarily long archives.
            f.index.seek(SeekFrom::Start(0))?;
            let mut found = false;
            for line in BufReader::new((&mut f.index).take(f.pending.index_len)).lines() {
                if let IndexEntry::Record {
                    epoch,
                    seq,
                    record_sha256,
                    ..
                } = serde_json::from_str(&line?)?
                {
                    if seq == record.seq && epoch == record.epoch {
                        if record_sha256 != record_hash {
                            bail!(
                                "conflicting duplicate Record {}/{}",
                                record.stream,
                                record.seq
                            );
                        }
                        found = true;
                        break;
                    }
                }
            }
            f.index.seek(SeekFrom::End(0))?;
            if !found {
                bail!(
                    "duplicate identity absent from file index: {}/{}",
                    record.stream,
                    record.seq
                );
            }
            return Ok(());
        }
        if record.seq != f.pending.cursor.next {
            bail!(
                "unannounced gap in {}: expected {}, received {}",
                record.stream,
                f.pending.cursor.next,
                record.seq
            );
        }
        let next = record
            .seq
            .checked_add(1)
            .context("record sequence exhausted u64 cursor space")?;
        let bytes = match self.format {
            Format::Raw => record.payload.clone(),
            Format::Jsonl => {
                let mut line = serde_json::to_vec(
                    &serde_json::json!({"format_version":SCHEMA_VERSION,"record":record}),
                )?;
                line.push(b'\n');
                line
            }
        };
        let entry = IndexEntry::Record {
            epoch: record.epoch.clone(),
            seq: record.seq,
            record_sha256: record_hash,
            offset: f.pending.confirmed_len,
            length: bytes.len() as u64,
        };
        crate::errorpoint("file_write")?;
        f.data.write_all(&bytes).context("write archive data")?;
        failpoint("file_after_write");
        f.data_hash.update(&bytes);
        f.pending.confirmed_len = f
            .pending
            .confirmed_len
            .checked_add(bytes.len() as u64)
            .context("file length overflow")?;
        append_index(f, &entry)?;
        f.pending.cursor.next = next;
        Ok(())
    }
    pub fn gap(&mut self, gap: &Gap, advances: bool) -> Result<()> {
        let f = self
            .streams
            .get_mut(&gap.stream)
            .context("gap for unconfigured stream")?;
        if gap.epoch != f.pending.cursor.epoch || gap.to < gap.from {
            bail!("invalid gap or epoch changed");
        }
        if gap.to < f.pending.cursor.next && advances {
            return Ok(());
        }
        if gap.from > f.pending.cursor.next {
            bail!("gap does not cover next expected record");
        }
        append_index(
            f,
            &IndexEntry::Gap {
                gap: gap.clone(),
                advances,
            },
        )?;
        f.pending.gap_count += 1;
        f.pending.incomplete = true;
        f.last_gap = Some(gap.clone());
        if advances {
            f.pending.cursor.next = gap
                .to
                .checked_add(1)
                .context("gap sequence exhausted cursor space")?
                .max(f.pending.cursor.next);
        }
        Ok(())
    }
    pub fn commit(&mut self) -> Result<()> {
        for f in self.streams.values_mut() {
            crate::errorpoint("file_sync")?;
            f.data.sync_all().context("sync archive file")?;
            f.index.sync_all().context("sync archive index")?;
            failpoint("file_after_sync");
            f.pending.confirmed_sha256 = digest(&f.data_hash);
            f.pending.index_sha256 = digest(&f.index_hash);
            crate::errorpoint("checkpoint_replace")?;
            save_checkpoint(&sibling(&f.path, ".checkpoint.json"), &f.pending)
                .context("persist file checkpoint")?;
            f.confirmed = f.pending.clone();
            failpoint("file_after_checkpoint");
        }
        Ok(())
    }
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({"streams":self.streams.iter().map(|(s,f)| (s.clone(),serde_json::json!({"confirmed":f.confirmed.cursor,"written":f.pending.cursor,"confirmed_bytes":f.confirmed.confirmed_len,"gap_count":f.confirmed.gap_count,"incomplete":f.confirmed.incomplete,"last_gap":f.last_gap}))).collect::<BTreeMap<_,_>>()})
    }
}
fn append_index(f: &mut StreamFile, entry: &IndexEntry) -> Result<()> {
    let mut bytes = serde_json::to_vec(entry)?;
    bytes.push(b'\n');
    f.index
        .write_all(&bytes)
        .context("write archive record index")?;
    f.index_hash.update(&bytes);
    f.pending.index_len = f
        .pending
        .index_len
        .checked_add(bytes.len() as u64)
        .context("index length overflow")?;
    Ok(())
}

fn validate_index(index: &mut File, checkpoint: &FileCheckpoint) -> Result<()> {
    index.seek(SeekFrom::Start(0))?;
    let mut cursor = checkpoint.initial.clone();
    let mut offset = 0u64;
    let mut gap_count = 0u64;
    for line in BufReader::new(index.take(checkpoint.index_len)).lines() {
        match serde_json::from_str::<IndexEntry>(&line?)? {
            IndexEntry::Record {
                epoch,
                seq,
                record_sha256,
                offset: entry_offset,
                length,
            } => {
                if epoch != cursor.epoch
                    || seq != cursor.next
                    || offset != entry_offset
                    || record_sha256.len() != 64
                    || !record_sha256.bytes().all(|v| v.is_ascii_hexdigit())
                {
                    bail!("file index is inconsistent with stream cursor or byte offsets");
                }
                cursor.next = seq.checked_add(1).context("file index sequence overflow")?;
                offset = offset
                    .checked_add(length)
                    .context("file index length overflow")?;
            }
            IndexEntry::Gap { gap, advances } => {
                if gap.stream != checkpoint.stream
                    || gap.epoch != cursor.epoch
                    || gap.from > cursor.next
                    || gap.to < gap.from
                {
                    bail!("file index gap is invalid");
                }
                gap_count += 1;
                if advances {
                    cursor.next = gap
                        .to
                        .checked_add(1)
                        .context("file index gap sequence overflow")?
                        .max(cursor.next);
                }
            }
        }
    }
    if cursor != checkpoint.cursor
        || offset != checkpoint.confirmed_len
        || gap_count != checkpoint.gap_count
        || checkpoint.incomplete != (gap_count > 0)
    {
        bail!("file checkpoint disagrees with its record index");
    }
    Ok(())
}
