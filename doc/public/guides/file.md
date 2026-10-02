<span id="读取日志文件"></span>

# Read a log file

`input-file` supports continuous following and fast static reads. It reads raw byte chunks without requiring a final newline. Save this complete configuration as `file.json`, replacing `path` with the absolute path of an existing regular file:

```json
{
  "plugins": [{
    "id": "file", "role": "input", "bin": "input-file",
    "streams": [{"id": "logs", "description": "Application log file"}],
    "config": {"path": "/absolute/path/app.log", "mode": "follow", "from_start": false}
  }]
}
```

```sh
./target/release/log-print --state .log-print/file.json start --config file.json
./target/release/log-print --state .log-print/file.json streams
```

Replace `STREAM_ID` below with the actual `id` UUID returned by `streams`:

```sh
./target/release/log-print --state .log-print/file.json read STREAM_ID --raw
./target/release/log-print --state .log-print/file.json plugin stop file
./target/release/log-print --state .log-print/file.json stop
```

`read` is a bounded snapshot, starting at the oldest currently retained record and returning up to 64 records by default. Repeated calls may repeat data; they are not a continuous subscription. Use an Output plugin for continuous processing or [archiving](archive.md).

<span id="选择读取方式"></span>

## Choose a mode

- Follow new bytes: `mode: "follow"`, `from_start: false`, starting at the file's end when opened.
- Follow and include existing content: `mode: "follow"`, `from_start: true`.
- Read a static file quickly: `mode: "static"`, always from the beginning, exiting at the first EOF.

In static mode, `source_eof` means the source has been read and transport-side publish calls have completed. It does **not** mean every downstream Output has saved the data. Core can overwrite unread records and Inputs do not wait for Outputs, so fast reads do not guarantee a complete import. When the plugin exits, Core and its retained stream remain until explicitly stopped.

Following detects replacement and truncation and starts a new segment from the beginning. If the path disappears, it waits. Old-file tails and changes between polls may be missed. Configuration is fixed at main-program startup; restart the instance after editing it. See [input-file](../plugins/input-file.md) for all fields and limits.
