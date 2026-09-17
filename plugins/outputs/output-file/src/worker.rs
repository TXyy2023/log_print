use crate::{
    checkpoint::{self, Cursor, FileCheckpoint, Gap},
    config::{Config, Mode},
    failpoint,
    file_sink::FileSink,
    sqlite_sink::SqliteSink,
};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use log_proto::Record;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    path::{Component, Path, PathBuf},
};

pub struct PreparedArchive {
    config: Config,
    locks: Vec<File>,
    config_digest: String,
}
pub struct Archive {
    config: Config,
    _locks: Vec<File>,
    file: Option<FileSink>,
    sqlite: Option<SqliteSink>,
    error: Option<String>,
    archive_id: String,
}
impl Archive {
    /// Claims all targets before the caller resolves live Core epochs/start positions.
    pub fn prepare(config: &Config) -> Result<PreparedArchive> {
        config.validate()?;
        let mut config = config.clone();
        if let Some(file) = &mut config.file {
            for path in file.paths.values_mut() {
                *path = absolute_path(path)?;
            }
        }
        if let Some(sqlite) = &mut config.sqlite {
            sqlite.path = absolute_path(&sqlite.path)?;
        }
        let mut targets = Vec::new();
        let mut managed = Vec::new();
        if let Some(file) = &config.file {
            for path in file.paths.values() {
                targets.push(path.clone());
                managed.push(path.clone());
                for suffix in [
                    ".records.jsonl",
                    ".checkpoint.json",
                    ".checkpoint.json.tmp",
                    ".lock",
                ] {
                    managed.push(checkpoint::sibling(path, suffix));
                }
            }
        }
        if let Some(sqlite) = &config.sqlite {
            targets.push(sqlite.path.clone());
            managed.push(sqlite.path.clone());
            for suffix in ["-wal", "-shm", "-journal", ".lock"] {
                managed.push(checkpoint::sibling(&sqlite.path, suffix));
            }
        }
        let mut paths = BTreeSet::new();
        let mut identities = BTreeSet::new();
        for path in &managed {
            let spelling = if cfg!(any(windows, target_os = "macos")) {
                path.to_string_lossy().to_lowercase()
            } else {
                path.to_string_lossy().into_owned()
            };
            if !paths.insert(spelling) {
                bail!(
                    "archive target/state/lock paths conflict: {}",
                    path.display()
                );
            }
            if let Ok(metadata) = std::fs::symlink_metadata(path) {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    bail!(
                        "archive managed path must be a regular non-symlink file: {}",
                        path.display()
                    );
                }
                if config.mode == Mode::Create {
                    bail!(
                        "create refuses existing target or state: {}",
                        path.display()
                    );
                }
                let id = checkpoint::identity(&File::open(path)?)?;
                if !identities.insert((id.volume, id.index)) {
                    bail!("archive managed files are hard-link aliases");
                }
            }
        }
        targets.sort();
        let mut locks = Vec::new();
        for path in targets {
            let lock_path = checkpoint::sibling(&path, ".lock");
            let file = if config.mode == Mode::Create {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&lock_path)
            } else {
                OpenOptions::new().read(true).write(true).open(&lock_path)
            }
            .with_context(|| {
                format!(
                    "open archive lock {} (partial initialization is not resumed)",
                    lock_path.display()
                )
            })?;
            file.try_lock_exclusive()
                .with_context(|| format!("archive target is already locked: {}", path.display()))?;
            file.sync_all()?;
            checkpoint::sync_parent(&lock_path)?;
            locks.push(file);
        }
        let config_digest = checkpoint::hex_hash(&serde_json::to_vec(
            &serde_json::json!({"streams":config.streams.iter().collect::<BTreeSet<_>>(),"file":config.file,"sqlite":config.sqlite}),
        )?);
        Ok(PreparedArchive {
            config,
            locks,
            config_digest,
        })
    }
    pub fn open(config: &Config, initial: BTreeMap<String, Cursor>) -> Result<Self> {
        Self::prepare(config)?.initialize(initial)
    }
    pub fn cursors(&self) -> BTreeMap<String, Cursor> {
        let mut cursors = self
            .file
            .as_ref()
            .map(FileSink::cursors)
            .unwrap_or_default();
        if let Some(sqlite) = &self.sqlite {
            for (stream, cursor) in sqlite.cursors() {
                cursors
                    .entry(stream)
                    .and_modify(|old| old.next = old.next.min(cursor.next))
                    .or_insert(cursor);
            }
        }
        cursors
    }
    fn healthy(&self) -> Result<()> {
        if let Some(error) = &self.error {
            bail!("archive stopped after error: {error}");
        }
        Ok(())
    }
    pub fn accept(&mut self, record: &Record) -> Result<()> {
        self.healthy()?;
        let result: Result<()> = (|| {
            if let Some(file) = &mut self.file {
                file.accept(record).context("file target")?;
            }
            if let Some(sqlite) = &mut self.sqlite {
                sqlite.accept(record).context("sqlite target")?;
            }
            Ok(())
        })();
        if let Err(error) = &result {
            self.error = Some(format!("{error:#}"));
        }
        result
    }
    pub fn gap(&mut self, gap: &Gap) -> Result<()> {
        self.healthy()?;
        let result: Result<()> = (|| {
            if let Some(file) = &mut self.file {
                file.gap(gap, !self.config.fail_on_gap)
                    .context("file target")?;
            }
            if let Some(sqlite) = &mut self.sqlite {
                sqlite
                    .gap(gap, !self.config.fail_on_gap)
                    .context("sqlite target")?;
            }
            Ok(())
        })();
        if let Err(error) = &result {
            self.error = Some(format!("{error:#}"));
        }
        result
    }
    pub fn commit(&mut self) -> Result<()> {
        self.healthy()?;
        let result: Result<()> = (|| {
            if let Some(file) = &mut self.file {
                file.commit().context("file target commit")?;
            }
            failpoint("between_targets");
            if let Some(sqlite) = &mut self.sqlite {
                sqlite.commit().context("sqlite target commit")?;
            }
            Ok(())
        })();
        if let Err(error) = &result {
            self.error = Some(format!("{error:#}"));
        }
        result
    }
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({"archive_id":self.archive_id,"state":if self.error.is_some(){"failed"}else{"active"},"error":self.error,"file":self.file.as_ref().map(FileSink::status),"sqlite":self.sqlite.as_ref().map(SqliteSink::status),"common":self.cursors()})
    }
}
impl PreparedArchive {
    pub fn initialize(self, initial: BTreeMap<String, Cursor>) -> Result<Archive> {
        let Self {
            config,
            locks,
            config_digest,
        } = self;
        let archive_id = if config.mode == Mode::Create {
            if initial.keys().collect::<BTreeSet<_>>() != config.streams.iter().collect()
                || initial.values().any(|c| c.epoch.is_empty() || c.next == 0)
            {
                bail!("all initial cursors require a resolved nonempty epoch and nonzero next");
            }
            uuid::Uuid::new_v4().to_string()
        } else if let Some(file) = &config.file {
            let path = file.paths.values().next().context("file paths empty")?;
            let checkpoint: FileCheckpoint = serde_json::from_reader(
                File::open(checkpoint::sibling(path, ".checkpoint.json"))
                    .context("initialization incomplete: checkpoint missing")?,
            )?;
            checkpoint.archive_id
        } else {
            let path = &config
                .sqlite
                .as_ref()
                .context("missing SQLite config")?
                .path;
            let connection = rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            connection
                .query_row(
                    "SELECT value FROM metadata WHERE key='archive_id'",
                    [],
                    |row| row.get(0),
                )
                .context("initialization incomplete: archive metadata missing")?
        };
        uuid::Uuid::parse_str(&archive_id).context("invalid archive identity")?;
        let file = config
            .file
            .as_ref()
            .map(|_| FileSink::open(&config, &initial, &archive_id, &config_digest))
            .transpose()
            .context("open file target")?;
        let sqlite = config
            .sqlite
            .as_ref()
            .map(|_| SqliteSink::open(&config, &initial, &archive_id, &config_digest))
            .transpose()
            .context("open sqlite target")?;
        if let (Some(file), Some(sqlite)) = (&file, &sqlite) {
            let sql_cursors = sqlite.cursors();
            for (stream, cursor) in file.cursors() {
                if sql_cursors
                    .get(&stream)
                    .is_none_or(|sql| sql.epoch != cursor.epoch)
                {
                    bail!("archive target epochs disagree");
                }
            }
        }
        Ok(Archive {
            config,
            _locks: locks,
            file,
            sqlite,
            error: None,
            archive_id,
        })
    }
}
fn absolute_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    bail!("path escapes filesystem root");
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let filename = normalized
        .file_name()
        .context("archive path must name a file")?
        .to_os_string();
    let parent = normalized
        .parent()
        .context("archive path must have a parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create archive parent {}", parent.display()))?;
    Ok(std::fs::canonicalize(parent)?.join(filename))
}
