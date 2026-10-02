<span id="配置参考"></span>

# Configuration reference

You can declare sources, outputs and fields with [CLI startup options](cli.md). JSON files are an optional batch configuration format loaded with `--config FILE`.

app-log-print reads command-line settings or the configuration file once at startup, saves the run's snapshot, and passes it to Core and plugins. Plugins started or restarted later use that same snapshot. Only stopping and starting the entire instance applies new startup settings. Unknown fields are rejected.

```json
{
  "core": {"transport":"tcp","buffer_records":4096,"buffer_bytes":4194304},
  "plugins": [
    {"id":"source","role":"input","bin":"input-file",
     "streams":[{"id":"logs","description":"Application logs"}],
     "config":{"path":"example.log","mode":"follow"}},
    {"id":"screen","role":"output","bin":"output-raw","autostart":false,
     "reads":["logs"],"config":{}}
  ]
}
```

<span id="core"></span>

## Core

| Field | Default and constraints |
| --- | --- |
| `transport` | `tcp`; optionally `udp` |
| `buffer_records` | 4096; 1..1000000 per stream |
| `buffer_bytes` | 4194304; 1..1073741824 bytes per stream, including serialized metadata |
| `max_payload_bytes` | 65536; 1..65536 |
| `read_batch_records` | 64; 1..64 |
| `queue_records` | 64; 1..1024 |

A single record larger than a stream's entire byte budget fails. TCP JSONL frames are limited to 1 MiB; encoded UDP datagrams to 60 KiB (JSON byte arrays expand the data). Official plugins default to 4096-byte chunks. UDP success only confirms local sending, not delivery of every record.

<span id="插件声明"></span>

## Plugin declaration

| Field | Meaning |
| --- | --- |
| `id` | Unique within the instance; at most 100 bytes; the internal `__` prefix is reserved |
| `role` | Required: `input` or `output` |
| `bin`, `args` | Executable and arguments; a relative path in a JSON config's bin is resolved from the config directory; bare names prefer adjacent built tools |
| `autostart` | true by default; false defers startup until `plugin start` |
| `streams` | At most one published stream; id is an alias, description at most 4096 bytes, parents declares derived sources |
| `reads` | Authorized source aliases or UUIDs for Outputs; Inputs do not subscribe |
| `read_all` | false by default; all-stream access for Outputs without owned streams or explicit reads |
| `config` | The plugin's static business configuration |

Each Input receives one Core-assigned stream, allocated during registration if not declared beforehand. A transforming Output may own a separate derived stream. Core enforces one writer and forbids self-subscription and configured feedback loops. Limits of 128 configured plugins and 128 configured subscriptions per plugin are resource budgets, not competing-consumer rules.

`save`, legacy storage settings and dynamic `config.patch` are no longer valid. Replace `input-replay` with input-file's `mode:static`. See [migration](../guides/recovery.md) and the plugin references.

<span id="全流只读-output-与-webui"></span>

## All-stream Outputs and WebUI

`read_all` is allowed only for an Output with no owned streams and no explicit reads. Such a consumer cannot create or publish streams. WebUI and its companion archive enable this permission; other plugin configurations retain their existing authorization rules.

WebUI `config.archive_dir` corresponds to `--webui-archive`; `config.history_plugin` corresponds to `--webui-history`. They are mutually exclusive. `config.state_path` selects a separate Page SQLite database. `config.listen` defaults to `127.0.0.1:0` and permits only loopback addresses. The supervisor generates each run's archive path and runtime_id. Cold startup restores display configuration and rebinds this run's streams.

<span id="终端显示配置"></span>

## Terminal display

`output-tui` uses the same Output fields and `read_all` constraints as `output-webui`. Set `bin` to `output-tui`; its `config` accepts `archive_dir` or `history_plugin`, `state_path` and loopback `listen`. Page databases default to `.tui/` under the instance state directory. They store configuration, not archived logs. See [the terminal workbench](../plugins/output-tui.md).
