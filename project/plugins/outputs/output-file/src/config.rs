use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Create,
    Resume,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Raw,
    Jsonl,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub format: Format,
    pub paths: BTreeMap<String, PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SqliteConfig {
    pub path: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CommitConfig {
    pub max_records: usize,
    pub max_bytes: usize,
    pub max_delay_ms: u64,
}
impl Default for CommitConfig {
    fn default() -> Self {
        Self {
            max_records: 64,
            max_bytes: 4 * 1024 * 1024,
            max_delay_ms: 100,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueueConfig {
    pub max_records: usize,
    pub max_bytes: usize,
}
impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_records: 256,
            max_bytes: 16 * 1024 * 1024,
        }
    }
}
fn default_from() -> u64 {
    1
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub streams: Vec<String>,
    #[serde(default = "default_from")]
    pub from: u64,
    pub mode: Mode,
    pub file: Option<FileConfig>,
    pub sqlite: Option<SqliteConfig>,
    #[serde(default)]
    pub commit: CommitConfig,
    #[serde(default)]
    pub queue: QueueConfig,
    #[serde(default = "yes")]
    pub fail_on_gap: bool,
}
impl Config {
    pub fn parse(value: serde_json::Value) -> Result<Self> {
        let config: Self = serde_json::from_value(value)?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        let streams: BTreeSet<_> = self.streams.iter().collect();
        if streams.is_empty()
            || streams.len() != self.streams.len()
            || self.streams.iter().any(|s| s.is_empty())
        {
            bail!("streams must be nonempty, unique nonempty names");
        }
        if self.file.is_none() && self.sqlite.is_none() {
            bail!("at least one archive target is required");
        }
        if let Some(file) = &self.file {
            if file.paths.keys().collect::<BTreeSet<_>>() != streams {
                bail!("file.paths must contain exactly the configured streams");
            }
            if file.paths.values().any(|p| p.as_os_str().is_empty()) {
                bail!("file paths must not be empty");
            }
        }
        if self
            .sqlite
            .as_ref()
            .is_some_and(|s| s.path.as_os_str().is_empty())
        {
            bail!("sqlite path must not be empty");
        }
        if !(1..=65536).contains(&self.commit.max_records)
            || !(1..=64 * 1024 * 1024).contains(&self.commit.max_bytes)
            || !(1..=60000).contains(&self.commit.max_delay_ms)
        {
            bail!("invalid commit limits (records 1..65536, bytes 1..64MiB, delay 1..60000ms)");
        }
        if !(1..=65536).contains(&self.queue.max_records)
            || !(log_proto::MAX_WIRE..=1024 * 1024 * 1024).contains(&self.queue.max_bytes)
        {
            bail!("invalid queue limits (records 1..65536, bytes 1MiB..1GiB)");
        }
        Ok(())
    }
    pub fn validate_reads(&self, reads: &[String]) -> Result<()> {
        if self.streams.iter().any(|stream| !reads.contains(stream)) {
            bail!("streams must be contained in plugin reads");
        }
        Ok(())
    }
}
