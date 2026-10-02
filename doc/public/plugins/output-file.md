<span id="output-file-保存日志"></span>

# output-file: archive logs

Subscribes to and saves logs without displaying payloads or changing Core's bounded memory behavior. See [Display and archive](../guides/archive.md).

```json
{
  "streams":["source"], "mode":"create", "fail_on_gap":true,
  "file":{"format":"jsonl","paths":{"source":"capture/source.jsonl"}},
  "sqlite":{"path":"capture/records.sqlite"},
  "commit":{"max_records":64,"max_bytes":4194304,"max_delay_ms":100},
  "queue":{"max_records":256,"max_bytes":16777216}
}
```

Choose at least one of `file` and `sqlite`. Each file stream needs a distinct path; data, indexes, checkpoints and locks must not alias one another. Create parent directories first. Existing targets are never overwritten. After CLI binding to a UUID, a single-stream configuration retains its one destination path.

| Field | Default and range |
| --- | --- |
| streams | Stream aliases/UUIDs; actual authorized reads take precedence |
| mode | Live operation supports only create; legacy resume does not apply to current Core |
| file.format | raw or jsonl |
| fail_on_gap | true; record an observed receive-sequence gap and fail; false records it and continues |
| commit.max_records | 64; 1–65536 |
| commit.max_bytes | 4 MiB; 1 byte–64 MiB |
| commit.max_delay_ms | 100; 1–60000 ms |
| queue.max_records | 256; 1–65536 |
| queue.max_bytes | 16 MiB; 1 MiB–1 GiB, measured as serialized size including metadata |

<span id="格式与提交"></span>

## Formats and commits

`raw` concatenates payloads without adding newlines, prefixes or record boundaries. Empty payloads still appear in the digest index. Each `jsonl` line is `{"format_version":2,"record":{...}}`; the payload is a byte array, preserving binary data.

New SQLite archives use schema 3 and store complete Records, a stream directory, dynamic stream registrations, and numeric sequence/observation-time indexes. The shared HistoryReader uses an independent read-only WAL connection and also supports schema 2; existing databases are not automatically migrated. Payloads are BLOBs; unsigned 64-bit sequences and nanosecond times are stored as decimal TEXT with indexed representations for queries. Derived relationships use JSON. Records include source `channel` and `source_seq`, but no Core persistence state. The unique key is `(stream,epoch,seq)`.

Receipt, writing and confirmed commit are separate stages. A dedicated worker serializes archive operations, committing by record count, bytes or elapsed wait. Files synchronize data and indexes before atomically replacing checkpoints; SQLite commits with WAL and synchronous=FULL. File and SQLite targets commit separately; dual-target writes are not an atomic transaction across both. A 100 ms threshold triggers a commit, not a guaranteed disk confirmation deadline.

Queue budgets include active writes. The SDK also has a fixed event queue and pending frames per subscription, so these budgets are not a process RSS ceiling. Slower downstream processing does not make Inputs wait; Core can still overwrite old records.

<span id="状态与停止"></span>

## Status and shutdown

`plugin call archive status.get` reports queues, per-target written/confirmed cursors, gaps and errors. `common[UUID].next` is the next sequence after all targets' confirmed prefix. Reaching the source's `head + 1` in the same epoch means it has caught up to that snapshot; an empty queue alone does not establish this.

`shutdown` closes SDK reception and cancels subscriptions, drains accepted events and the worker queue, commits targets and reports status before replying `stopped:true`. Data not yet in the SDK queue is outside the accepted set. Plugin shutdown does not imply Input completion or a fully drained pipeline. Disk, sync, database, validation or connection errors do not return complete success.

A timeout means the result is unknown and cleanup may still be running. Check process and status before starting another writer for the same destination.

<span id="历史与版本边界"></span>

## History and version boundaries

Core atomically selects the first subscription position as its oldest retained record; the subscription then waits continuously for new data. The live plugin has no arbitrary historical starting cursor, automatic reconnect or old-epoch recovery. Observed local sequence jumps are recorded as gaps. Absence of observed gaps is not proof of end-to-end completeness.

With `read_all`, a SQLite archive can follow all current and newly derived streams. WebUI/TUI companion archives set `fail_on_gap=false` to continue after a gap; explicitly configured archives keep the user's setting. [WebUI history](output-webui.md) queries combine committed archive data and a fixed Core memory range while reporting actual coverage.

The archive library retains checkpoint validation/recovery logic and tests, but the live plugin does not expose resume. Upgrades use new directories and preserve old files and sidecars. Power-loss behavior depends on the filesystem and hardware; these tests are not physical power-failure certification.

Configuration is the main process's startup snapshot and changes require a main-process restart. Validate with `cargo test -p output-file` and `python3 quality/tests/v2/outputs.py`; retired historical suites are not current acceptance evidence.
