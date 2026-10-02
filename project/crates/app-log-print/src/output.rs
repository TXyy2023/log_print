//! Human-readable CLI rendering; the wire protocol remains independent.
use anyhow::{Context, Result};
use serde_json::Value;
use std::fmt::Write;

fn text(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "none".into(),
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        Value::String(value) if value.is_empty() => "(empty)".into(),
        Value::String(value) => text(value),
        Value::Number(value) => value.to_string(),
        _ => "none".into(),
    }
}

fn tree(out: &mut String, label: &str, value: &Value, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        Value::Object(map) if !map.is_empty() => {
            if !label.is_empty() {
                let _ = writeln!(out, "{pad}{}:", text(label));
            }
            for (key, value) in map {
                tree(
                    out,
                    key,
                    value,
                    indent + if label.is_empty() { 0 } else { 2 },
                );
            }
        }
        Value::Array(items) if !items.is_empty() => {
            let _ = writeln!(out, "{pad}{}:", text(label));
            for (i, item) in items.iter().enumerate() {
                tree(out, &format!("- {}", i + 1), item, indent + 2);
            }
        }
        _ => {
            let _ = writeln!(out, "{pad}{}: {}", text(label), scalar(value));
        }
    }
}

pub fn details(value: &Value) -> String {
    let mut out = String::new();
    tree(
        &mut out,
        if value.is_object() { "" } else { "Result" },
        value,
        0,
    );
    out
}

pub fn streams(value: &Value) -> Result<String> {
    let streams = value.as_array().context("stream reply is not a list")?;
    if streams.is_empty() {
        return Ok("No streams.\n".into());
    }
    let mut out = "UUID\tOWNER\tRECORDS\tBYTES\tDESCRIPTION\n".to_owned();
    for stream in streams {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}",
            scalar(&stream["id"]),
            scalar(&stream["owner"]),
            scalar(&stream["buffer_records"]),
            scalar(&stream["buffer_bytes"]),
            scalar(&stream["description"])
        );
    }
    Ok(out)
}

pub fn read(page: &Value) -> Result<String> {
    let records = page["records"]
        .as_array()
        .context("read reply has no records")?;
    let mut metadata = page.clone();
    metadata
        .as_object_mut()
        .context("read reply is not an object")?
        .remove("records");
    let mut out = details(&metadata);
    let _ = writeln!(out, "Records: {}", records.len());
    for value in records {
        let record: log_proto::Record = serde_json::from_value(value.clone())?;
        let mut metadata = value.clone();
        metadata
            .as_object_mut()
            .context("record is not an object")?
            .remove("payload");
        let _ = writeln!(
            out,
            "\nRecord #{} ({} bytes)",
            record.seq,
            record.payload.len()
        );
        out.push_str(&details(&metadata));
        match std::str::from_utf8(&record.payload) {
            Ok(payload) => {
                let _ = writeln!(out, "payload: {}", text(payload));
            }
            Err(_) => {
                out.push_str("payload (hex):");
                for byte in record.payload {
                    let _ = write!(out, " {byte:02x}");
                }
                out.push('\n');
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn results_and_control_characters_are_readable() {
        let result =
            details(&json!({"success":false,"forced":true,"name":"中文\n\u{1b}[2J","items":[1,2]}));
        assert!(result.contains("success: no"));
        assert!(result.contains("forced: yes"));
        assert!(result.contains("中文\\n\\u{1b}[2J"));
        assert!(!result.contains('\u{1b}'));
        assert!(serde_json::from_str::<Value>(&result).is_err());
        assert_eq!(streams(&json!([])).unwrap(), "No streams.\n");
    }

    #[test]
    fn binary_read_displays_metadata_and_hex() {
        let record = json!({"stream":"uuid","epoch":"epoch","seq":2,"key":"key",
            "payload":[0,255,10],"channel":"stderr","source_seq":7,"observed_ts_ns":1,
            "upstream":{},"upstream_epochs":{}});
        let rendered = read(&json!({"stream":"uuid","records":[record]})).unwrap();
        assert!(rendered.contains("channel: stderr"));
        assert!(rendered.contains("source_seq: 7"));
        assert!(rendered.contains("payload (hex): 00 ff 0a"));
    }
}
