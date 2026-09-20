use anyhow::{bail, Result};
use encoding_rs::{Decoder, Encoding};
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub streams: Vec<String>,
    pub output_stream: String,
    pub from: u64,
    pub input_encoding: String,
    pub output_encoding: String,
    pub radix: String,
    pub split_lines: bool,
    pub keep_newline: bool,
    pub flush_partial: bool,
    pub prefix: String,
    pub suffix: String,
    pub delete: Vec<String>,
    pub replace: Vec<Replacement>,
    pub max_pending_bytes: usize,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Replacement {
    pub pattern: String,
    pub with: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            streams: vec![],
            output_stream: "derived".into(),
            from: 1,
            input_encoding: "auto".into(),
            output_encoding: "utf-8".into(),
            radix: "none".into(),
            split_lines: false,
            keep_newline: true,
            flush_partial: true,
            prefix: String::new(),
            suffix: String::new(),
            delete: vec![],
            replace: vec![],
            max_pending_bytes: 65536,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.streams.is_empty()
            || self.output_stream.is_empty()
            || self.streams.contains(&self.output_stream)
        {
            bail!("nonempty parent streams, separate output_stream required")
        }
        io_plugin_util::bounded("max_pending_bytes", self.max_pending_bytes, 4, 1024 * 1024)?;
        for label in [&self.input_encoding, &self.output_encoding] {
            if !["auto", "raw"].contains(&label.as_str())
                && Encoding::for_label(label.as_bytes()).is_none()
            {
                bail!("unsupported encoding {label}")
            }
        }
        if self.output_encoding == "auto" {
            bail!("output_encoding must be explicit")
        }
        if ![
            "none",
            "hex_encode",
            "hex_decode",
            "bin_encode",
            "bin_decode",
            "dec_encode",
            "dec_decode",
        ]
        .contains(&self.radix.as_str())
        {
            bail!("invalid radix mode")
        }
        io_plugin_util::bounded("rule_count", self.delete.len() + self.replace.len(), 0, 64)?;
        for d in &self.delete {
            io_plugin_util::bounded("pattern_bytes", d.len(), 0, 4096)?;
            Regex::new(d)?;
        }
        for r in &self.replace {
            io_plugin_util::bounded("pattern_bytes", r.pattern.len(), 0, 4096)?;
            io_plugin_util::bounded("replacement_bytes", r.with.len(), 0, self.max_pending_bytes)?;
            Regex::new(&r.pattern)?;
        }
        if self.prefix.len() + self.suffix.len() > self.max_pending_bytes {
            bail!("prefix/suffix exceed pending bound")
        }
        Ok(())
    }
}

pub struct Processor {
    decoder: Option<Decoder>,
    line: Vec<u8>,
    token: Vec<u8>,
    automatic: bool,
    raw: bool,
}
pub struct Output {
    pub bytes: Vec<Vec<u8>>,
    pub warnings: Vec<String>,
}
impl Processor {
    pub fn new(c: &Config) -> Self {
        Self {
            decoder: Encoding::for_label(c.input_encoding.as_bytes()).map(|e| e.new_decoder()),
            line: vec![],
            token: vec![],
            automatic: c.input_encoding == "auto",
            raw: c.input_encoding == "raw",
        }
    }
    pub fn pending(&self) -> usize {
        self.line.len() + self.token.len()
    }
    fn radix_decode(&mut self, input: &[u8], last: bool, c: &Config) -> Result<Vec<u8>> {
        let base = match c.radix.as_str() {
            "hex_decode" => 16,
            "bin_decode" => 2,
            "dec_decode" => 10,
            _ => return Ok(input.to_vec()),
        };
        let mut output = Vec::new();
        for b in input
            .iter()
            .copied()
            .chain(if last { Some(b' ') } else { None })
        {
            if b.is_ascii_whitespace() {
                if !self.token.is_empty() {
                    let s = std::str::from_utf8(&self.token)?;
                    let s = match base {
                        16 => s.strip_prefix("0x").unwrap_or(s),
                        2 => s.strip_prefix("0b").unwrap_or(s),
                        _ => s,
                    };
                    output.push(u8::from_str_radix(s, base)?);
                    self.token.clear();
                }
            } else {
                self.token.push(b);
                if self.token.len() > 16 {
                    bail!("numeric byte token exceeds 16 bytes")
                }
            }
        }
        Ok(output)
    }
    pub fn feed(&mut self, input: &[u8], last: bool, c: &Config) -> Result<Output> {
        let input = self.radix_decode(input, last, c)?;
        let mut warnings = vec![];
        let data = if let Some(decoder) = &mut self.decoder {
            let capacity = decoder
                .max_utf8_buffer_length(input.len())
                .ok_or_else(|| anyhow::anyhow!("decode capacity overflow"))?;
            let mut text = String::with_capacity(capacity);
            let (result, read, errors) = decoder.decode_to_string(&input, &mut text, last);
            if errors {
                bail!("invalid bytes in manual input encoding; original stream remains intact")
            }
            if result == encoding_rs::CoderResult::OutputFull || read != input.len() {
                bail!("bounded decode output exhausted")
            };
            text.into_bytes()
        } else {
            if self.automatic && !input.is_empty() && std::str::from_utf8(&input).is_err() {
                let mut detector = chardetng::EncodingDetector::new();
                detector.feed(&input, last);
                let (guess, assessed) = detector.guess_assess(None, true);
                warnings.push(format!("encoding_uncertain: likely {}, assessed={}; keeping original bytes; set input_encoding manually to convert",guess.name(),assessed));
            }
            input
        };
        let mut units = vec![];
        if c.split_lines {
            // Consume incrementally so many short lines in one record never exceed the bound.
            for b in data {
                self.line.push(b);
                if self.line.len() > c.max_pending_bytes {
                    bail!("incomplete line exceeds max_pending_bytes")
                };
                if b == b'\n' {
                    let mut line = std::mem::take(&mut self.line);
                    if !c.keep_newline {
                        line.pop();
                        if line.last() == Some(&b'\r') {
                            line.pop();
                        }
                    }
                    units.push(line);
                }
            }
            if last && !self.line.is_empty() {
                if c.flush_partial {
                    units.push(std::mem::take(&mut self.line))
                } else {
                    warnings.push(format!(
                        "partial_line_not_emitted: {} bytes",
                        self.line.len()
                    ));
                    self.line.clear();
                }
            }
        } else if !data.is_empty() {
            units.push(data)
        }
        let mut output = Vec::new();
        for unit in units {
            // Automatic uncertainty must not turn invalid data into replacements or apply text edits.
            if self.automatic && std::str::from_utf8(&unit).is_err() {
                output.push(radix_encode(unit, &c.radix));
                continue;
            }
            let mut edited = unit;
            for pattern in &c.delete {
                edited = replace_bounded(
                    &Regex::new(pattern)?,
                    &edited,
                    b"",
                    c.max_pending_bytes.saturating_mul(4),
                )?;
            }
            for replacement in &c.replace {
                edited = replace_bounded(
                    &Regex::new(&replacement.pattern)?,
                    &edited,
                    replacement.with.as_bytes(),
                    c.max_pending_bytes.saturating_mul(4),
                )?;
            }
            if edited.len() + c.prefix.len() + c.suffix.len()
                > c.max_pending_bytes.saturating_mul(4)
            {
                bail!("edited unit exceeds expansion bound")
            }
            let mut wrapped = Vec::with_capacity(edited.len() + c.prefix.len() + c.suffix.len());
            wrapped.extend_from_slice(c.prefix.as_bytes());
            wrapped.extend(edited);
            wrapped.extend_from_slice(c.suffix.as_bytes());
            if !self.raw && c.output_encoding != "raw" {
                let text = std::str::from_utf8(&wrapped)?;
                let encoding = Encoding::for_label(c.output_encoding.as_bytes())
                    .ok_or_else(|| anyhow::anyhow!("unknown target encoding"))?;
                if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
                    wrapped = text
                        .encode_utf16()
                        .flat_map(|u| {
                            if encoding == encoding_rs::UTF_16LE {
                                u.to_le_bytes()
                            } else {
                                u.to_be_bytes()
                            }
                        })
                        .collect();
                } else {
                    let (bytes, _, errors) = encoding.encode(text);
                    if errors {
                        bail!(
                            "output encoding cannot represent input; no replacement bytes emitted"
                        )
                    };
                    wrapped = bytes.into_owned();
                }
            }
            output.push(radix_encode(wrapped, &c.radix));
        }
        Ok(Output {
            bytes: output,
            warnings,
        })
    }
}
fn replace_bounded(
    regex: &Regex,
    input: &[u8],
    replacement: &[u8],
    limit: usize,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut end = 0;
    for captures in regex.captures_iter(input) {
        let m = captures.get(0).unwrap();
        let mut expanded = Vec::new();
        captures.expand(replacement, &mut expanded);
        if output.len() + m.start() - end + expanded.len() > limit {
            bail!("replacement exceeds expansion bound")
        }
        output.extend_from_slice(&input[end..m.start()]);
        output.extend(expanded);
        end = m.end();
    }
    if output.len() + input.len() - end > limit {
        bail!("replacement exceeds expansion bound")
    }
    output.extend_from_slice(&input[end..]);
    Ok(output)
}
fn radix_encode(bytes: Vec<u8>, mode: &str) -> Vec<u8> {
    match mode {
        "hex_encode" => bytes
            .iter()
            .map(|b| format!("{b:02x} "))
            .collect::<String>()
            .into_bytes(),
        "bin_encode" => bytes
            .iter()
            .map(|b| format!("{b:08b} "))
            .collect::<String>()
            .into_bytes(),
        "dec_encode" => bytes
            .iter()
            .map(|b| format!("{b} "))
            .collect::<String>()
            .into_bytes(),
        _ => bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn defaults() -> Config {
        Config {
            streams: vec!["a".into()],
            ..Config::default()
        }
    }
    #[test]
    fn split_utf8_and_edit_across_chunks() {
        let c = Config {
            input_encoding: "utf-8".into(),
            split_lines: true,
            prefix: "[".into(),
            suffix: "]".into(),
            replace: vec![Replacement {
                pattern: "temp".into(),
                with: "T".into(),
            }],
            ..defaults()
        };
        let mut p = Processor::new(&c);
        assert!(p
            .feed(&[b't', b'e', b'm', b'p', b'=', 0xe4], false, &c)
            .unwrap()
            .bytes
            .is_empty());
        let out = p.feed(&[0xb8, 0xad, b'\n', b'z'], false, &c).unwrap();
        assert_eq!(out.bytes, ["[T=中\n]".as_bytes()]);
        assert_eq!(p.feed(b"", true, &c).unwrap().bytes, [b"[z]"]);
    }
    #[test]
    fn uncertain_preserves_invalid() {
        let c = defaults();
        let mut p = Processor::new(&c);
        let o = p.feed(&[0xff, 0x00, 0xfe], false, &c).unwrap();
        assert_eq!(o.bytes, [vec![0xff, 0x00, 0xfe]]);
        assert!(!o.warnings.is_empty());
    }
    #[test]
    fn manual_shift_jis() {
        let c = Config {
            input_encoding: "shift_jis".into(),
            ..defaults()
        };
        let mut p = Processor::new(&c);
        assert!(p.feed(&[0x93], false, &c).unwrap().bytes.is_empty());
        assert_eq!(p.feed(&[0xfa], false, &c).unwrap().bytes, ["日".as_bytes()]);
    }
    #[test]
    fn numeric_roundtrip_chunk_boundary() {
        for (enc, dec) in [
            ("hex_encode", "hex_decode"),
            ("bin_encode", "bin_decode"),
            ("dec_encode", "dec_decode"),
        ] {
            let e = Config {
                input_encoding: "raw".into(),
                radix: enc.into(),
                ..defaults()
            };
            let mut p = Processor::new(&e);
            let text = p.feed(&[0, 42, 255], false, &e).unwrap().bytes.concat();
            let d = Config {
                input_encoding: "raw".into(),
                radix: dec.into(),
                ..defaults()
            };
            let mut q = Processor::new(&d);
            let mut out = vec![];
            for b in text {
                out.extend(q.feed(&[b], false, &d).unwrap().bytes.concat())
            }
            out.extend(q.feed(b"", true, &d).unwrap().bytes.concat());
            assert_eq!(out, [0, 42, 255]);
        }
    }
    #[test]
    fn bounded_line_and_invalid_encoding() {
        let c = Config {
            split_lines: true,
            max_pending_bytes: 4,
            ..defaults()
        };
        assert!(Processor::new(&c).feed(b"12345", false, &c).is_err());
        let c = Config {
            input_encoding: "utf-8".into(),
            ..defaults()
        };
        assert!(Processor::new(&c).feed(&[0xff], true, &c).is_err());
    }
}
