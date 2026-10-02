<span id="完整性与-0-1-2-迁移"></span>

# Integrity and migration to 0.1.2

Core buffers are memory-only. Stopping or restarting Core creates new stream identities and cannot restore the previous buffers. If an Input exits while Core keeps running, its stream and retained data remain.

<span id="保存结果"></span>

## Persistence results

output-file saves the records it actually receives to raw files, JSONL or SQLite. Its checkpoints and commit state do not let Core recover missing history. Normal shutdown drains the accepted queue and checks writes and commits. Inspect `plugin stop` results, including `success`, `forced` and plugin reports. Forced termination or failure does not mean persistence succeeded.

The live plugin accepts only `mode:create` and does not restore an archive cursor from Core. Existing destinations are never overwritten. Preserve old archives and sidecar state, using compatible tools to read them. Newly created SQLite archives use schema 3; the shared HistoryReader can also read schema 2. Archives contain no Core durability state and are not automatically migrated.

<span id="旧配置迁移"></span>

## Migrate old configurations

| 0.1.1 | 0.1.2 / current source |
| --- | --- |
| `log-print/1` | `log-print/2`; old-client handshakes fail explicitly |
| Core/plugin/stream `save` | Removed; configure output-file for persistence |
| Multiple streams per Input | One stream per Input; stdout/stderr use channels |
| input-replay | input-file with `mode:static` |
| Configured stream id is the runtime ID | Configuration IDs are aliases; use Core's returned UUID |
| `config set` / `config.patch` | Edit configuration, then restart the entire instance |
| `read --from/--epoch`, historical recovery | `read` returns retained snapshots; Outputs subscribe continuously |
| Complex encoding/editing transforms | Numbering, timestamps and bounded source-sequence reordering |

The current WebUI/TUI history engine combines optional output-file archives with a fixed Core memory boundary. This does not change the behavior of `read` or add disk storage to Core.

Lossless fast import, simultaneous completion of every Output, reliable UDP and automatic SDK reconnection are not guaranteed. Preserve source data, old archives and current runtime state separately.

Stop commands return actual cleanup results. If any plugin fails or is forcefully terminated, the CLI exits nonzero; restart does not launch a replacement after failed shutdown. A hung Unix plugin killed with SIGKILL cannot execute internal cleanup. In that case, source-process cleanup is unconfirmed, as is complete persistence or process-tree reclamation.
