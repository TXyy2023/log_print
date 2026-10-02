<span id="转换日志"></span>

# Transform logs

`output-transform` adds numbering or timestamps, or performs bounded source-sequence reordering, and publishes to a separate derived stream. Other Outputs can still display or archive the original stream.

<span id="加编号和纳秒时间戳"></span>

## Add numbering and nanosecond timestamps

Create `example.log` in the repository root and save this as `transform.json`:

```json
{
  "plugins":[
    {"id":"file","role":"input","bin":"input-file","streams":[{"id":"source"}],
     "config":{"path":"example.log","mode":"follow","from_start":true}},
    {"id":"transform","role":"output","bin":"output-transform","reads":["source"],
     "streams":[{"id":"derived","parents":["source"]}],
     "config":{"number":true,"timestamp":true}},
    {"id":"display","role":"output","bin":"output-raw","reads":["derived"],
     "config":{"annotate":false}}
  ]
}
```

```sh
./target/release/log-print --state .log-print/transform.json start --config transform.json
python3 -c "open('example.log','ab').write(b'temperature=23.5\n')"
./target/release/log-print --state .log-print/transform.json streams
```

Find the actual source and derived UUIDs in `streams`, then inspect each with `read <UUID>`. The derived payload looks like `[n=1] [ts_ns=...] temperature=23.5`; the source retains the original bytes. Numbering applies to input chunks, which are not necessarily individual lines.

<span id="有界重排"></span>

## Bounded reordering

Add `"reorder":true,"max_records":128,"max_bytes":4194304,"max_delay_ms":100`. Records are ordered by the Input's `source_seq`, separately for each stream and channel. A missing sequence starts a wait based on the oldest pending record's receive time. On expiry or impending buffer exhaustion, the plugin skips the missing numbers and continues. The first duplicate wins. Actual publication also depends on scheduling and transport latency.

This handles observable source reordering but cannot restore lost UDP packets or evicted Core data. It does not guarantee global source-timestamp order. stdout and stderr retain separate source ordering.

<span id="停止"></span>

## Stop

```sh
./target/release/log-print --state .log-print/transform.json plugin call transform shutdown
./target/release/log-print --state .log-print/transform.json status
./target/release/log-print --state .log-print/transform.json stop
```

Shutdown drains accepted data and pending reordered records. A `stopping` reply means shutdown has begun; use `status` to confirm the final stopped state. Confirm downstream archive commits separately. See [output-transform](../plugins/output-transform.md) for defaults and limits.
