pub mod checkpoint;
pub mod config;
pub mod file_sink;
pub mod sqlite_sink;
pub mod worker;
pub use checkpoint::{Cursor, Gap};
pub use config::{Config, Format, Mode};
pub use worker::{Archive, PreparedArchive};

/// Includes serialized metadata and JSON byte-array expansion, not just payload bytes.
pub fn record_bytes(record: &log_proto::Record) -> anyhow::Result<usize> {
    Ok(serde_json::to_vec(record)?.len())
}

pub(crate) fn failpoint(name: &str) {
    if std::env::var("LOG_PRINT_ARCHIVE_TESTING").as_deref() == Ok("1")
        && std::env::var("LOG_PRINT_ARCHIVE_FAILPOINT").as_deref() == Ok(name)
    {
        std::process::abort();
    }
}

/// Explicitly opt-in error injection for subprocess acceptance tests; never a
/// business configuration option and never active without the testing guard.
pub(crate) fn errorpoint(name: &str) -> anyhow::Result<()> {
    if std::env::var("LOG_PRINT_ARCHIVE_TESTING").as_deref() == Ok("1")
        && std::env::var("LOG_PRINT_ARCHIVE_ERRORPOINT").as_deref() == Ok(name)
    {
        anyhow::bail!("injected archive I/O failure at {name}; durability not confirmed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::sibling;
    use log_proto::Record;
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        fs::{self, OpenOptions},
        io::Write,
        path::PathBuf,
    };
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("output-file-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn config(&self, mode: &str) -> Config {
            let file = if mode != "sqlite" {
                Some(json!({"format":"raw","paths":{"s":self.0.join("s.raw")}}))
            } else {
                None
            };
            let sqlite = if mode != "file" {
                Some(json!({"path":self.0.join("archive.sqlite")}))
            } else {
                None
            };
            Config::parse(json!({"streams":["s"],"mode":"create","file":file,"sqlite":sqlite}))
                .unwrap()
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn initial(next: u64) -> BTreeMap<String, Cursor> {
        BTreeMap::from([(
            "s".into(),
            Cursor {
                epoch: "e".into(),
                next,
            },
        )])
    }
    fn record(seq: u64, payload: &[u8]) -> Record {
        Record {
            stream: "s".into(),
            epoch: "e".into(),
            seq,
            key: "k\n\0中文".into(),
            payload: payload.to_vec(),
            source_ts_ns: Some(u64::MAX),
            observed_ts_ns: u64::MAX,
            upstream: BTreeMap::from([("origin".into(), u64::MAX)]),
            upstream_epochs: BTreeMap::from([("origin".into(), "other epoch".into())]),
            channel: Some("stdout".into()),
            source_seq: Some(u64::MAX),
        }
    }
    fn resume(mut config: Config) -> Config {
        config.mode = Mode::Resume;
        config
    }
    #[test]
    fn all_targets_roundtrip_exact_record_and_empty_payload() {
        for mode in ["file", "sqlite", "both"] {
            let temp = Temp::new();
            let config = temp.config(mode);
            let records = [
                record(1, &[0, 255, 0xe4]),
                record(2, &[0xb8, 0xad]),
                record(3, b""),
            ];
            let mut archive = Archive::open(&config, initial(1)).unwrap();
            for record in &records {
                archive.accept(record).unwrap();
            }
            assert_eq!(archive.cursors()["s"].next, 1);
            archive.commit().unwrap();
            assert_eq!(archive.cursors()["s"].next, 4);
            drop(archive);
            let mut archive = Archive::open(&resume(config.clone()), BTreeMap::new()).unwrap();
            for record in &records {
                archive.accept(record).unwrap();
            }
            archive.commit().unwrap();
            assert_eq!(archive.cursors()["s"].next, 4);
            drop(archive);
            if mode != "sqlite" {
                assert_eq!(
                    fs::read(temp.0.join("s.raw")).unwrap(),
                    vec![0, 255, 0xe4, 0xb8, 0xad]
                );
            }
            if mode != "file" {
                let db = rusqlite::Connection::open(temp.0.join("archive.sqlite")).unwrap();
                let count: u64 = db
                    .query_row("SELECT count(*) FROM records", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(count, 3);
                let timestamp: String = db
                    .query_row("SELECT observed_ts_ns FROM records LIMIT 1", [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(timestamp, u64::MAX.to_string());
            }
        }
    }
    #[test]
    fn full_metadata_conflicts_are_rejected_even_for_empty_payload() {
        for mode in ["file", "sqlite", "both"] {
            let temp = Temp::new();
            let config = temp.config(mode);
            let mut archive = Archive::open(&config, initial(1)).unwrap();
            archive.accept(&record(1, b"")).unwrap();
            archive.commit().unwrap();
            drop(archive);
            let mut archive = Archive::open(&resume(config), BTreeMap::new()).unwrap();
            let mut conflicting = record(1, b"");
            conflicting.channel = Some("stderr".into());
            assert!(archive
                .accept(&conflicting)
                .unwrap_err()
                .to_string()
                .contains("target"));
            assert!(archive.commit().is_err());
        }
    }
    #[test]
    fn unconfirmed_file_tail_and_sql_transaction_are_recovered() {
        let temp = Temp::new();
        let config = temp.config("both");
        let mut archive = Archive::open(&config, initial(1)).unwrap();
        archive.accept(&record(1, b"confirmed")).unwrap();
        archive.commit().unwrap();
        archive.accept(&record(2, b"partial")).unwrap();
        drop(archive);
        let mut archive = Archive::open(&resume(config), BTreeMap::new()).unwrap();
        assert_eq!(archive.cursors()["s"].next, 2);
        assert_eq!(fs::read(temp.0.join("s.raw")).unwrap(), b"confirmed");
        archive.accept(&record(2, b"partial")).unwrap();
        archive.commit().unwrap();
        assert_eq!(fs::read(temp.0.join("s.raw")).unwrap(), b"confirmedpartial");
    }
    #[test]
    fn external_changes_and_file_identity_replacement_are_rejected_without_truncation() {
        for change in ["same_length", "shorter", "replace"] {
            let temp = Temp::new();
            let config = temp.config("file");
            let path = temp.0.join("s.raw");
            let mut archive = Archive::open(&config, initial(1)).unwrap();
            archive.accept(&record(1, b"abc")).unwrap();
            archive.commit().unwrap();
            drop(archive);
            match change {
                "same_length" => fs::write(&path, b"abdTAIL").unwrap(),
                "shorter" => fs::write(&path, b"a").unwrap(),
                _ => {
                    fs::rename(&path, temp.0.join("old")).unwrap();
                    fs::write(&path, b"abc").unwrap();
                }
            }
            let before = fs::read(&path).unwrap();
            assert!(Archive::open(&resume(config), BTreeMap::new()).is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }
    #[test]
    fn sqlite_deleted_or_modified_rows_fail_logical_integrity_check() {
        for sql in [
            "DELETE FROM records WHERE seq='1'",
            "UPDATE records SET payload=X'00' WHERE seq='1'",
            "UPDATE checkpoints SET next='4'",
            "UPDATE records SET seq='01' WHERE seq='1'",
        ] {
            let temp = Temp::new();
            let config = temp.config("sqlite");
            let mut archive = Archive::open(&config, initial(1)).unwrap();
            archive.accept(&record(1, b"abc")).unwrap();
            archive.accept(&record(2, b"def")).unwrap();
            archive.commit().unwrap();
            drop(archive);
            let db = rusqlite::Connection::open(temp.0.join("archive.sqlite")).unwrap();
            db.execute_batch(sql).unwrap();
            let integrity: String = db
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                .unwrap();
            assert_eq!(integrity, "ok");
            drop(db);
            assert!(
                Archive::open(&resume(config), BTreeMap::new()).is_err(),
                "accepted tampering: {sql}"
            );
        }
    }
    #[test]
    fn gap_is_durable_and_only_continuation_advances_cursor() {
        for advances in [false, true] {
            let temp = Temp::new();
            let mut config = temp.config("both");
            config.fail_on_gap = !advances;
            let mut archive = Archive::open(&config, initial(1)).unwrap();
            archive
                .gap(&Gap {
                    stream: "s".into(),
                    epoch: "e".into(),
                    from: 1,
                    to: 3,
                    reason: "history evicted".into(),
                })
                .unwrap();
            archive.commit().unwrap();
            assert_eq!(archive.cursors()["s"].next, if advances { 4 } else { 1 });
            drop(archive);
            let mut archive = Archive::open(&resume(config), BTreeMap::new()).unwrap();
            assert_eq!(archive.status()["sqlite"]["incomplete"], true);
            if advances {
                archive.accept(&record(4, b"after gap")).unwrap();
                archive.commit().unwrap();
            }
        }
    }
    #[test]
    fn target_exclusivity_and_create_refusal_preserve_files() {
        let temp = Temp::new();
        let config = temp.config("both");
        let archive = Archive::open(&config, initial(1)).unwrap();
        assert!(Archive::open(&resume(config.clone()), BTreeMap::new()).is_err());
        assert!(Archive::open(&config, initial(1)).is_err());
        drop(archive);
        assert!(Archive::open(&resume(config), BTreeMap::new()).is_ok());
    }
    #[test]
    fn path_aliases_and_unknown_configuration_fail() {
        let temp = Temp::new();
        let mut config = temp.config("both");
        config.sqlite.as_mut().unwrap().path = temp.0.join("s.raw.checkpoint.json");
        assert!(Archive::prepare(&config).is_err());
        assert!(Config::parse(
            json!({"streams":["s"],"mode":"create","sqlite":{"path":"x","typo":1}})
        )
        .is_err());
        assert!(
            Config::parse(json!({"streams":["s","s"],"mode":"create","sqlite":{"path":"x"}}))
                .is_err()
        );
        assert!(Config::parse(
            json!({"streams":["s"],"mode":"create","sqlite":{"path":"x"},"queue":{"max_bytes":1}})
        )
        .is_err());
    }
    #[test]
    fn initial_cursors_and_archive_identity_are_required() {
        let temp = Temp::new();
        let config = temp.config("both");
        let prepared = Archive::prepare(&config).unwrap();
        assert!(prepared.initialize(initial(0)).is_err());
        assert!(Archive::open(&resume(config), BTreeMap::new()).is_err());
        let temp = Temp::new();
        let config = temp.config("both");
        let archive = Archive::open(&config, initial(123)).unwrap();
        drop(archive);
        let mut changed = resume(config);
        changed.file.as_mut().unwrap().format = Format::Jsonl;
        assert!(Archive::open(&changed, BTreeMap::new()).is_err());
    }
    #[test]
    fn jsonl_encodes_complete_record_losslessly() {
        let temp = Temp::new();
        let mut config = temp.config("file");
        config.file.as_mut().unwrap().format = Format::Jsonl;
        let mut archive = Archive::open(&config, initial(1)).unwrap();
        let record = record(1, &[0, 255, 10]);
        archive.accept(&record).unwrap();
        archive.commit().unwrap();
        drop(archive);
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(temp.0.join("s.raw")).unwrap()).unwrap();
        assert_eq!(value["format_version"], 2);
        assert_eq!(
            serde_json::from_value::<Record>(value["record"].clone()).unwrap(),
            record
        );
    }
    #[test]
    fn u64_sequence_boundary_is_explicit_and_lossless() {
        let temp = Temp::new();
        let config = temp.config("both");
        let mut archive = Archive::open(&config, initial(u64::MAX - 1)).unwrap();
        archive.accept(&record(u64::MAX - 1, b"x")).unwrap();
        archive.commit().unwrap();
        drop(archive);
        let mut archive = Archive::open(&resume(config), BTreeMap::new()).unwrap();
        assert_eq!(archive.cursors()["s"].next, u64::MAX);
        assert!(archive.accept(&record(u64::MAX, b"x")).is_err());
    }
    #[test]
    fn unconfirmed_index_tail_is_removed_but_confirmed_tampering_is_rejected() {
        let temp = Temp::new();
        let config = temp.config("file");
        let path = temp.0.join("s.raw");
        let mut archive = Archive::open(&config, initial(1)).unwrap();
        archive.accept(&record(1, b"data")).unwrap();
        archive.commit().unwrap();
        drop(archive);
        let index = sibling(&path, ".records.jsonl");
        let before = fs::read(&index).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&index)
            .unwrap()
            .write_all(b"partial index")
            .unwrap();
        let archive = Archive::open(&resume(config.clone()), BTreeMap::new()).unwrap();
        drop(archive);
        assert_eq!(fs::read(&index).unwrap(), before);
        let mut changed = before;
        changed[0] = b'!';
        fs::write(&index, changed).unwrap();
        assert!(Archive::open(&resume(config), BTreeMap::new()).is_err());
    }
    #[test]
    fn record_byte_budget_counts_metadata_and_binary_expansion() {
        let mut value = record(1, &[255; 64 * 1024]);
        value.key = "m".repeat(32000);
        assert!(record_bytes(&value).unwrap() > value.payload.len() + value.key.len());
    }
}
