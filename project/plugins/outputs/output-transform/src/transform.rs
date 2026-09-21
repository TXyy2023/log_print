use anyhow::{bail, Context, Result};
use log_proto::Record;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub streams: Vec<String>,
    pub output_stream: String,
    pub number: bool,
    pub timestamp: bool,
    pub reorder: bool,
    pub max_records: usize,
    pub max_bytes: usize,
    pub max_delay_ms: u64,
    pub max_channels: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            streams: vec![],
            output_stream: String::new(),
            number: false,
            timestamp: false,
            reorder: false,
            max_records: 128,
            max_bytes: 4 * 1024 * 1024,
            max_delay_ms: 100,
            max_channels: 128,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.streams.iter().any(|s| s.is_empty()) {
            bail!("stream names must not be empty");
        }
        if !(1..=4096).contains(&self.max_records)
            || !(log_proto::MAX_PAYLOAD..=64 * 1024 * 1024).contains(&self.max_bytes)
            || !(1..=60000).contains(&self.max_delay_ms)
            || !(1..=4096).contains(&self.max_channels)
        {
            bail!("invalid bounded reorder limits");
        }
        Ok(())
    }
}
type Channel = (String, String, Option<String>);
struct Queued {
    record: Record,
    received: Instant,
    bytes: usize,
}
#[derive(Default)]
struct State {
    next: u64,
    pending: BTreeMap<u64, Queued>,
}
/// Counters are local diagnostics. They do not imply that missing source records can be recovered.
#[derive(Default, Debug)]
pub struct Stats {
    pub duplicates: u64,
    pub skipped: u64,
    pub missing_source_seq: u64,
}
pub struct Processor {
    config: Config,
    channels: BTreeMap<Channel, State>,
    records: usize,
    bytes: usize,
    pub stats: Stats,
    number: u64,
    source_sequences: BTreeMap<Option<String>, u64>,
}
impl Processor {
    pub fn new(config: &Config) -> Self {
        Self {
            config: config.clone(),
            channels: BTreeMap::new(),
            records: 0,
            bytes: 0,
            stats: Stats::default(),
            number: 0,
            source_sequences: BTreeMap::new(),
        }
    }
    pub fn pending(&self) -> (usize, usize) {
        (self.records, self.bytes)
    }
    fn release(&mut self, key: &Channel, force: bool) -> Vec<Record> {
        let mut output = Vec::new();
        let state = self.channels.get_mut(key).unwrap();
        if force {
            if let Some((&first, _)) = state.pending.first_key_value() {
                self.stats.skipped = self
                    .stats
                    .skipped
                    .saturating_add(first.saturating_sub(state.next));
                state.next = first;
            }
        }
        while let Some(item) = state.pending.remove(&state.next) {
            state.next = state.next.saturating_add(1);
            self.records -= 1;
            self.bytes -= item.bytes;
            output.push(item.record);
        }
        output
    }
    fn oldest(&self) -> Option<Channel> {
        self.channels
            .iter()
            .filter_map(|(key, state)| {
                state
                    .pending
                    .values()
                    .map(|v| v.received)
                    .min()
                    .map(|time| (key, time))
            })
            .min_by_key(|(_, time)| *time)
            .map(|(key, _)| key.clone())
    }
    pub fn feed(&mut self, record: Record, now: Instant) -> Result<Vec<Record>> {
        if !self.config.reorder {
            return Ok(vec![record]);
        }
        let Some(seq) = record.source_seq else {
            self.stats.missing_source_seq += 1;
            return Ok(vec![record]);
        };
        if seq == 0 || seq == u64::MAX {
            bail!("source sequence must be 1..u64::MAX-1");
        }
        let key = (
            record.stream.clone(),
            record.epoch.clone(),
            record.channel.clone(),
        );
        if !self.channels.contains_key(&key) {
            if self.channels.len() >= self.config.max_channels {
                bail!("source channel state limit exceeded");
            }
            self.channels.insert(
                key.clone(),
                State {
                    next: 1,
                    pending: BTreeMap::new(),
                },
            );
        }
        let state = &self.channels[&key];
        if seq < state.next || state.pending.contains_key(&seq) {
            self.stats.duplicates += 1;
            return Ok(vec![]);
        }
        if seq == state.next {
            self.channels.get_mut(&key).unwrap().next += 1;
            let mut output = vec![record];
            output.extend(self.release(&key, false));
            return Ok(output);
        }
        let bytes = serde_json::to_vec(&record)?.len();
        if bytes > self.config.max_bytes {
            bail!("record exceeds reorder byte budget");
        }
        let mut output = Vec::new();
        while self.records >= self.config.max_records || self.bytes + bytes > self.config.max_bytes
        {
            let oldest = self.oldest().context("invalid reorder budget accounting")?;
            output.extend(self.release(&oldest, true));
        }
        // A pressure flush can advance this channel beyond the incoming record.
        if seq < self.channels[&key].next {
            self.stats.duplicates += 1;
            return Ok(output);
        }
        self.channels.get_mut(&key).unwrap().pending.insert(
            seq,
            Queued {
                record,
                received: now,
                bytes,
            },
        );
        self.records += 1;
        self.bytes += bytes;
        output.extend(self.release(&key, false));
        Ok(output)
    }
    pub fn expire(&mut self, now: Instant) -> Vec<Record> {
        let mut output = Vec::new();
        while let Some(key) = self.oldest() {
            let oldest = self.channels[&key]
                .pending
                .values()
                .map(|q| q.received)
                .min()
                .unwrap();
            if now.saturating_duration_since(oldest)
                < Duration::from_millis(self.config.max_delay_ms)
            {
                break;
            }
            output.extend(self.release(&key, true));
        }
        output
    }
    pub fn flush(&mut self) -> Vec<Record> {
        let mut output = Vec::new();
        while let Some(key) = self.oldest() {
            output.extend(self.release(&key, true));
        }
        output
    }
    pub fn next_source_sequence(&mut self, channel: Option<String>) -> Result<u64> {
        let next = self.source_sequences.entry(channel).or_default();
        *next = next
            .checked_add(1)
            .context("derived channel sequence exhausted")?;
        Ok(*next)
    }
    pub fn decorate(&mut self, record: &Record) -> Result<Vec<u8>> {
        self.number = self
            .number
            .checked_add(1)
            .context("transform numbering exhausted")?;
        let mut output = Vec::new();
        if self.config.number {
            output.extend_from_slice(format!("[n={}] ", self.number).as_bytes());
        }
        if self.config.timestamp {
            output.extend_from_slice(
                format!(
                    "[ts_ns={}] ",
                    record.source_ts_ns.unwrap_or(record.observed_ts_ns)
                )
                .as_bytes(),
            );
        }
        output.extend_from_slice(&record.payload);
        if output.len() > log_proto::MAX_PAYLOAD {
            bail!("transformed record exceeds maximum payload; use smaller input chunks");
        }
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn record(seq: u64) -> Record {
        serde_json::from_value(serde_json::json!({"stream":"original","epoch":"e","seq":seq,"key":"key","payload":[0,255,10],"observed_ts_ns":42,"source_seq":seq,"upstream":{},"upstream_epochs":{}})).unwrap()
    }
    fn config() -> Config {
        Config {
            reorder: true,
            ..Config::default()
        }
    }
    fn seq(records: Vec<Record>) -> Vec<u64> {
        records.into_iter().map(|r| r.source_seq.unwrap()).collect()
    }
    #[test]
    fn source_reordering_preserves_original_content() {
        let mut p = Processor::new(&config());
        let now = Instant::now();
        assert!(p.feed(record(3), now).unwrap().is_empty());
        assert_eq!(seq(p.feed(record(1), now).unwrap()), vec![1]);
        let out = p.feed(record(2), now).unwrap();
        assert_eq!(out[0], record(2));
        assert_eq!(seq(out), vec![2, 3]);
        assert_eq!(p.pending(), (0, 0));
    }
    #[test]
    fn exact_capacity_does_not_drop_the_missing_record_when_it_arrives() {
        let mut p = Processor::new(&Config {
            max_records: 2,
            ..config()
        });
        let now = Instant::now();
        p.feed(record(2), now).unwrap();
        p.feed(record(3), now).unwrap();
        assert_eq!(seq(p.feed(record(1), now).unwrap()), vec![1, 2, 3]);
        assert_eq!(p.stats.skipped, 0);
    }
    #[test]
    fn derived_channel_sequences_are_independent() {
        let mut p = Processor::new(&config());
        assert_eq!(p.next_source_sequence(Some("out".into())).unwrap(), 1);
        assert_eq!(p.next_source_sequence(Some("err".into())).unwrap(), 1);
        assert_eq!(p.next_source_sequence(Some("out".into())).unwrap(), 2);
    }
    #[test]
    fn gaps_flush_after_window_and_late_duplicates_are_dropped() {
        let c = config();
        let mut p = Processor::new(&c);
        let now = Instant::now();
        p.feed(record(4), now).unwrap();
        assert!(p.expire(now + Duration::from_millis(99)).is_empty());
        assert_eq!(seq(p.expire(now + Duration::from_millis(100))), vec![4]);
        assert_eq!(p.stats.skipped, 3);
        assert!(p.feed(record(2), now).unwrap().is_empty());
        assert_eq!(p.stats.duplicates, 1);
    }
    #[test]
    fn pending_duplicate_first_wins_and_stop_flushes_sorted() {
        let mut p = Processor::new(&config());
        let now = Instant::now();
        p.feed(record(3), now).unwrap();
        let mut duplicate = record(3);
        duplicate.payload = b"different".to_vec();
        p.feed(duplicate, now).unwrap();
        p.feed(record(2), now).unwrap();
        let out = p.flush();
        assert_eq!(out[1].payload, record(3).payload);
        assert_eq!(seq(out), vec![2, 3]);
        assert_eq!(p.stats.duplicates, 1);
        assert_eq!(p.stats.skipped, 1);
    }
    #[test]
    fn pressure_is_bounded_and_channels_have_independent_source_order() {
        let c = Config {
            max_records: 2,
            ..config()
        };
        let mut p = Processor::new(&c);
        let now = Instant::now();
        p.feed(record(3), now).unwrap();
        p.feed(record(5), now).unwrap();
        assert_eq!(seq(p.feed(record(7), now).unwrap()), vec![3]);
        assert!(p.pending().0 <= 2);
        let mut a = record(1);
        a.channel = Some("stdout".into());
        let mut b = record(1);
        b.channel = Some("stderr".into());
        assert_eq!(
            p.feed(a, now).unwrap().last().unwrap().channel.as_deref(),
            Some("stdout")
        );
        assert_eq!(
            p.feed(b, now).unwrap().last().unwrap().channel.as_deref(),
            Some("stderr")
        );
    }
    #[test]
    fn numbering_and_timestamp_have_explicit_fallback_without_mutation() {
        let c = Config {
            number: true,
            timestamp: true,
            ..Config::default()
        };
        let mut p = Processor::new(&c);
        let r = record(1);
        assert_eq!(
            p.decorate(&r).unwrap(),
            [b"[n=1] [ts_ns=42] ".as_slice(), &r.payload].concat()
        );
        assert_eq!(r.payload, vec![0, 255, 10]);
        let mut next = r.clone();
        next.source_ts_ns = Some(99);
        assert!(p.decorate(&next).unwrap().starts_with(b"[n=2] [ts_ns=99] "));
    }
    #[test]
    fn rejects_oversize_payload_and_excess_channel_states() {
        let c = Config {
            number: true,
            max_channels: 1,
            ..config()
        };
        let mut p = Processor::new(&c);
        let now = Instant::now();
        p.feed(record(1), now).unwrap();
        let mut other = record(1);
        other.channel = Some("other".into());
        assert!(p.feed(other, now).is_err());
        let mut large = record(2);
        large.payload = vec![0; log_proto::MAX_PAYLOAD];
        assert!(p.decorate(&large).is_err());
    }
}
