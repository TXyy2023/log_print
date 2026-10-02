<span id="output-raw-终端日志展示"></span>

# output-raw: terminal log output

Subscribes to Core streams and writes to the plugin's stdout. Use [output-file](output-file.md) for persistence and [output-transform](output-transform.md) to modify content.

```json
{"streams":["source"],"annotate":false}
```

| Field | Behavior |
| --- | --- |
| streams | Optional aliases/UUIDs; main-config `reads` or actual CLI bindings take precedence |
| annotate | false by default; true prefixes each record with stream UUID, Core sequence and source channel |

By default, payload bytes are written without added newlines, including binary and empty payloads. Multiple streams interleave in this plugin's receive order, without a global time-order guarantee. The main program may redirect stdout; consult startup status for the actual location.

Subscriptions begin at Core's oldest retained record, then wait for new data. An idle or departed Input does not end the Output normally. Stop it explicitly. Buffer eviction and UDP loss can omit data; terminal output does not guarantee completeness.

`path`, `paths`, `from`, `append` and `overwrite` are not accepted. Configuration is the main program's startup snapshot, available through `config.get`. Restart the main program after edits; `config.patch` returns restart_required.
