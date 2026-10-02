use log_proto::Record;
use serde_json::{json, Value};
use std::collections::BTreeMap;
#[derive(Default)]
pub struct Lines {
    pending: BTreeMap<(String, String, String), Pending>,
    last: BTreeMap<String, u64>,
}
struct Pending {
    bytes: Vec<u8>,
    location: Value,
    end_seq: u64,
    end_offset: usize,
}
fn location(r: &Record, offset: usize) -> Value {
    json!({"stream":r.stream,"epoch":r.epoch,"seq":r.seq.to_string(),"offset":offset,"channel":r.channel,"key":r.key,"time":(r.observed_ts_ns/1_000_000).to_string(),"observed_ts_ns":r.observed_ts_ns.to_string(),"source_ts_ns":r.source_ts_ns.map(|n|n.to_string()),"source_seq":r.source_seq.map(|n|n.to_string()),"upstream":r.upstream.iter().map(|(k,v)|(k,v.to_string())).collect::<BTreeMap<_,_>>(),"upstream_epochs":r.upstream_epochs})
}
fn finish(p: Pending, partial: bool, gap: bool) -> Value {
    let mut value = p.location;
    value["kind"] = json!("line");
    value["text"] = json!(String::from_utf8_lossy(&p.bytes).trim_end_matches('\r'));
    value["hex"] = json!(p
        .bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" "));
    value["end_seq"] = json!(p.end_seq.to_string());
    value["end_offset"] = json!(p.end_offset);
    value["partial"] = json!(partial);
    value["gap"] = json!(gap);
    value
}
impl Lines {
    pub fn push(&mut self, r: &Record) -> Vec<Value> {
        let mut rows = Vec::new();
        if self
            .last
            .insert(r.stream.clone(), r.seq)
            .is_some_and(|n| n.saturating_add(1) != r.seq)
        {
            let keys: Vec<_> = self
                .pending
                .keys()
                .filter(|k| k.0 == r.stream)
                .cloned()
                .collect();
            for key in keys {
                rows.push(finish(self.pending.remove(&key).unwrap(), true, true));
            }
            rows.push(json!({"stream":r.stream,"epoch":r.epoch,"seq":r.seq.to_string(),"offset":0,"time":(r.observed_ts_ns/1_000_000).to_string(),"observed_ts_ns":r.observed_ts_ns.to_string(),"text":"[record gap]","kind":"gap","gap":true}));
        }
        let key = (
            r.stream.clone(),
            r.epoch.clone(),
            r.channel.clone().unwrap_or_default(),
        );
        for (offset, byte) in r.payload.iter().enumerate() {
            let p = self.pending.entry(key.clone()).or_insert_with(|| Pending {
                bytes: Vec::new(),
                location: location(r, offset),
                end_seq: r.seq,
                end_offset: offset,
            });
            p.end_seq = r.seq;
            p.end_offset = offset + 1;
            if *byte == b'\n' {
                rows.push(finish(self.pending.remove(&key).unwrap(), false, false));
            } else {
                p.bytes.push(*byte);
                if p.bytes.len() >= 64 * 1024 {
                    rows.push(finish(self.pending.remove(&key).unwrap(), true, false));
                }
            }
        }
        rows
    }
    pub fn flush(self) -> Vec<Value> {
        self.pending
            .into_values()
            .map(|p| finish(p, true, false))
            .collect()
    }
}
pub fn matches(row: &Value, settings: &Value, regex: Option<&regex::Regex>) -> bool {
    let channels = settings["channels"].as_array();
    if channels.is_some_and(|v| !v.is_empty() && !v.iter().any(|c| c == &row["channel"])) {
        return false;
    }
    let text = row["text"].as_str().unwrap_or("");
    if settings["text"].as_str().is_some_and(|v| !text.contains(v)) {
        return false;
    }
    if regex.is_some_and(|v| !v.is_match(text)) {
        return false;
    }
    in_time_window(row, settings)
}
fn in_time_window(row: &Value, settings: &Value) -> bool {
    let time = row["observed_ts_ns"]
        .as_str()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    time_bound(&settings["time_from"]).is_none_or(|v| time >= v)
        && time_bound(&settings["time_end"]).is_none_or(|v| time <= v)
}
pub fn curve_matches(row: &Value, settings: &Value) -> bool {
    if row["gap"] == true {
        in_time_window(row, settings)
    } else {
        matches(row, settings, None)
    }
}
pub fn time_bound(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
pub fn numeric(row: &Value, settings: &Value, regex: Option<&regex::Regex>) -> Option<f64> {
    if row["gap"] == true {
        return None;
    }
    let text = row["text"].as_str()?;
    let value = if let Some(field) = settings["field"].as_str().filter(|s| !s.is_empty()) {
        let mut value: Value = serde_json::from_str(text).ok()?;
        for key in field.split('.') {
            value = value.get(key)?.clone();
        }
        value.as_f64().or_else(|| value.as_str()?.parse().ok())?
    } else {
        regex?
            .captures(text)?
            .name("value")?
            .as_str()
            .parse()
            .ok()?
    };
    value.is_finite().then_some(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn record(seq: u64, channel: &str, bytes: &[u8]) -> Record {
        Record {
            stream: "s".into(),
            epoch: "e".into(),
            seq,
            key: "".into(),
            payload: bytes.to_vec(),
            observed_ts_ns: 1,
            source_ts_ns: None,
            upstream: BTreeMap::new(),
            upstream_epochs: BTreeMap::new(),
            channel: Some(channel.into()),
            source_seq: None,
        }
    }
    #[test]
    fn split_channels_offsets_utf8() {
        let mut l = Lines::default();
        assert!(l.push(&record(1, "stdout", &[0xe4, 0xb8])).is_empty());
        let rows = l.push(&record(2, "stderr", b"bad\n"));
        assert_eq!(rows[0]["text"], "bad");
        let rows = l.push(&record(3, "stdout", &[0xad, b'\n', b'2', b'\n']));
        assert_eq!(rows[0]["text"], "中");
        assert_eq!(rows[0]["seq"], "1");
        assert_eq!(rows[0]["end_seq"], "3");
        assert_eq!(rows[1]["offset"], 2);
    }
    #[test]
    fn extraction_and_gap() {
        let mut l = Lines::default();
        l.push(&record(1, "stdout", b"val"));
        let rows = l.push(&record(3, "stdout", b"ue=3\n"));
        assert!(rows.iter().any(|r| r["gap"] == true));
        let re = regex::Regex::new("value=(?P<value>[0-9]+)").unwrap();
        assert_eq!(
            numeric(&json!({"text":"value=4"}), &json!({}), Some(&re)),
            Some(4.)
        );
        assert_eq!(
            numeric(
                &json!({"text":"{\"a\":{\"b\":5}}"}),
                &json!({"field":"a.b"}),
                None
            ),
            Some(5.)
        );
    }
}
