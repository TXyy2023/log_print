<span id="input-file-跟随与静态读取"></span>

# input-file: follow and static reads

[Guide](../guides/file.md) · [Common configuration](../reference/configuration.md)

Each instance writes one Core-assigned stream. `streams[0].id` is a configured alias; Core returns the actual UUID. Set the description in `streams[0].description`. This is the plugin's `config` object only:

```json
{"path":"/absolute/path/source.log","mode":"follow","from_start":false,"chunk_bytes":4096,"poll_ms":50}
```

| Field | Default and range | Behavior |
|---|---|---|
| `path` | Required | An existing regular file |
| `mode` | `follow` | Continuous following, or `static` to read quickly from the beginning |
| `from_start` | `false` | Follow mode only; true includes preexisting content |
| `chunk_bytes` | 4096; 1–65536 | Maximum chunk bytes; encoded UDP packet limits also apply |
| `poll_ms` | 50; 1–60000 | Follow polling interval in milliseconds |

Configuration is read once by the main program. Editing the file or restarting only a plugin does not reload it; restart the main program. There are no dynamic business settings. The plugin accepts `shutdown` and `config.get`, and fails when a Core connection error is detected. UDP lacks TCP EOF notification; the supervisor detects a Core process crash and coordinates cleanup.

<span id="文件边界"></span>

## File boundaries

By default, `follow` determines the file's end when opening it, before registering with Core. It reads appended bytes. A changed file identity, shortened length or changed anchor of up to 64 bytes before the read position starts a new segment from the beginning. A missing path is reported and awaited. Record `key` includes run ID, segment number and segment offset. `source_seq` increments from 1 per published chunk; `channel` is absent.

Rotation does not drain the old file's tail. Multiple replacements between polls, or truncation followed by restoration of the same anchor bytes, may go undetected. Rotation also depends on the writer's and operating system's file APIs. Bytes not yet published at shutdown are not guaranteed to arrive.

<span id="静态完成含义"></span>

## Static completion

`static` always reads from the beginning, publishes raw bytes as quickly as possible, and reports `source_eof` before exiting at the first EOF. It replaces the old input-replay static-read use case, without timing, speed multipliers, timestamp parsing or SQLite replay.

`bytes_sent` / `chunks_sent` count completed transport-side publish calls. TCP returns Core's acceptance result; UDP confirms only local sending. `downstream_complete` is always `false`: finishing the source does not mean Outputs are done. Core retains its stream and remaining buffer, but bounded memory overwrites old data and Inputs do not wait for Outputs. **Fast static reads do not guarantee a complete, lossless import.**

The old `config.stream` field is removed. See [configuration](../reference/configuration.md) for capacity and UDP limits.
