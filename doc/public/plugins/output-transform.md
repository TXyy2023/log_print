<span id="output-transform-加工与派生流"></span>

# output-transform: derived streams

Reads source streams as an Output and publishes transformed records to a separate single-writer stream. Original payloads and records remain unchanged. See [Transform logs](../guides/transform.md).

```json
{"streams":["source"],"output_stream":"derived","number":true,"timestamp":true,"reorder":true,"max_records":128,"max_bytes":4194304,"max_delay_ms":100,"max_channels":128}
```

The main configuration must declare `role:"output"`, authorized `reads` and a separate derived `streams` declaration. Actual read bindings override config.streams. The derived stream cannot be a source stream; knowing another UUID does not grant write access.

| Field | Default and behavior |
| --- | --- |
| streams / output_stream | Optional; use the authorized read and derived stream UUIDs from registration |
| number | false; prefix `[n=1] `, increasing from 1 within this plugin run |
| timestamp | false; prefix `[ts_ns=...] `, preferring source nanoseconds, otherwise Core observation time |
| reorder | false; true enables bounded source-sequence reordering |
| max_records | 128; total reorder-buffer records 1–4096 |
| max_bytes | 4 MiB; serialized reorder-buffer size 64 KiB–64 MiB |
| max_delay_ms | 100; expiry based on the oldest pending record's receive time, 1–60000 ms |
| max_channels | 128; source-state and derived-channel counters each limited to 1–4096; exceeding either fails explicitly in every mode |

With all transforms disabled, payloads are copied to the derived stream unchanged. Numbering and timestamps still produce one output per input record. Exceeding the protocol payload limit fails explicitly; reduce the input chunk size. The plugin neither splits records implicitly nor executes arbitrary user code.

<span id="重排、重复与缺号"></span>

## Reordering, duplicates and missing sequences

The reorder key is `(stream,epoch,channel)`, with an expected `source_seq` starting at 1 per group. stdout/stderr have independent source sequences and are not assumed to share an original ordering. Records without `source_seq` are published immediately in arrival order and counted.

The first duplicate wins; late sequences before the published position are dropped. Missing numbers cause wakeup at the pending record's actual deadline, not periodic polling. On expiry or impending budget exhaustion, the smallest pending sequence and subsequent contiguous records are published, counting skipped numbers. Normal shutdown stops acceptance before draining accepted events and reorder buffers. Deadlines trigger processing; actual publication also depends on scheduling and transport. Missing or evicted data cannot be recovered.

Each derived record's `upstream` points only to its actual source stream and Core sequence. It does not invent progress for other streams. The channel is preserved, with a new `source_seq` increasing from 1 per output channel. The numbering prefix counts publications across the entire transform instance.

Subscriptions begin at the oldest retained record and keep waiting at the end. Input EOF is not an automatic shutdown request. Configuration changes require restarting the main program. Legacy encoding, regex replacement and dynamic configuration features are not available.
