<span id="读取与管理实例"></span>

# Read and manage an instance

```sh
log-print streams
log-print stream STREAM_UUID
log-print read STREAM_UUID --limit 64
log-print read STREAM_UUID --raw --wait-ms 1000
```

Use the `id` returned by `streams`, not a configuration alias. `read` returns a snapshot of retained memory. Repeated calls may duplicate records and are not a lossless incremental query. For an empty stream, `--wait-ms` waits until the deadline or data arrival; a nonempty stream returns immediately.

For continuous consumption, configure an Output with `autostart:false`, then start it after the stream exists:

```sh
log-print plugin start screen --stream STREAM_UUID
log-print plugin stop screen
```

Each Output starts at the oldest retained record and waits for new data at the end. Stopping an Output does not close the stream or stop its Input. Buffers remain after the Input exits. `status` reports both plugin business state and actual child-process state; acceptance of a stop request is not proof of process completion.

Queries return tables and indented text. `read` includes metadata and log content, displaying non-text payload as hexadecimal. AI agents should preserve stream IDs, source sequence numbers and channel information. Neither a local UDP send nor Core's memory acceptance proves persistence. See the [CLI reference](../reference/cli.md).
