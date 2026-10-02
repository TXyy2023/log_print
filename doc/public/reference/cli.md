<span id="cli-参考"></span>

# CLI reference

```text
log-print [--state STATE] COMMAND
```

The default state path is `.log-print/state.json`. Results are tables or indented text, with booleans shown as `yes` / `no`. Failures report to stderr and exit nonzero. `read` displays record metadata and payload text, escaping control characters and displaying non-UTF-8 bytes as hexadecimal. `read --raw` emits exact payload bytes.

| Command | Purpose |
| --- | --- |
| `run [startup options]` | Run in the foreground; Ctrl-C stops owned processes |
| `start [startup options]` | Start in the background; return PID, log paths and stream IDs after readiness |
| `status` | Core, plugin reports and child-process exit state |
| `streams` | List stream UUIDs, descriptions, owners and buffer state |
| `stream UUID` | Inspect one stream |
| `resolve ALIAS` | Resolve a configured alias to an actual stream UUID |
| `describe UUID DESCRIPTION` | Change a stream's display description |
| `read UUID [--limit 64] [--wait-ms 0] [--raw]` | Retained-buffer snapshot; limit 1..64; empty results may wait 0..60000 ms |
| `config [--plugin ID]` | Read the startup snapshot used by this instance |
| `plugin start ID [--stream UUID]` | Start a configured plugin; optionally bind an unconnected Output to a stream |
| `plugin stop ID` | Request cleanup, wait for child exit and report success/forced |
| `plugin restart ID` | Stop, then restart with the same startup snapshot |
| `plugin call ID METHOD [KEY=VALUE ...]` | Invoke a plugin method, such as `config.get` or output-file's `status.get` |
| `call OP [KEY=VALUE ...]` | Advanced Core object/RPC operation |
| `stop` | Stop the instance and wait for removal of its state file |

<span id="纯命令行启动"></span>

## Start without a configuration file

Each plugin has an ID; an Input's default stream alias is its ID. Outputs normally subscribe to all declared Inputs. Select particular sources or derived streams with `--read OUTPUT_ID=ALIAS`. WebUI/TUI and companion archives use all-stream read access.

```sh
log-print start --input-file source=./application.log \
  --set source.from_start=true --describe 'source=Application logs' \
  --output-raw screen --no-autostart screen
log-print streams
log-print read STREAM_UUID --wait-ms 1000
log-print plugin start screen --stream STREAM_UUID
log-print stop
```

| Startup option (repeatable) | Meaning |
| --- | --- |
| `--input-file ID=PATH` | Follow a file; use `--set ID.mode=static` for a static read |
| `--input-program ID=COMMAND` | Launch a process; add arguments with `--list-text ID.args=ARG` |
| `--input-tmux ID=PANE` | Attach to an existing tmux pane |
| `--output-raw ID` | Continuously display source bytes |
| `--output-transform ID` | Declare a derived stream with the same alias; configure with e.g. `--set ID.number=true` |
| `--output-file ID=PATH` | Save one stream to a new raw file; use `--read` to select one when there are multiple Inputs |
| `--output-sqlite ID=PATH` | Save selected streams to a new SQLite database |
| `--output-webui ID` / `--output-tui ID` | Start a browser or terminal workbench backend |
| `--webui-archive ID=DIR` / `--tui-archive ID=DIR` | Create a companion all-stream SQLite archive for this run |
| `--webui-history ID=ARCHIVE` / `--tui-history ID=ARCHIVE` | Bind a configured SQLite archive plugin in this instance; mutually exclusive with the corresponding archive option |
| `--input ID=BINARY` / `--output ID=BINARY` | Declare a custom plugin; set business fields with the options below |
| `--core KEY=VALUE` | Set Core fields, e.g. `transport=udp` or `buffer_records=8192` |
| `--set ID.KEY=VALUE` | Set a plugin business field; dots address nested fields, e.g. `archive.commit.max_delay_ms=50` |
| `--text ID.KEY=VALUE` | Force a string, e.g. `source.env.FLAG=false` |
| `--list ID.KEY=VALUE` / `--list-text ID.KEY=VALUE` | Append a list item; the latter forces a string |
| `--read ID=ALIAS` | Select an Output source; repeat for multiple sources |
| `--describe ID=TEXT` | Set a published stream's description |
| `--no-autostart ID` | Defer startup until `plugin start` |
| `--plugin-arg ID=ARG` | Add an argument to the plugin executable itself |

`start` / `run` without options creates an empty instance. Paths are relative to the calling working directory, including a custom BINARY containing a relative path.

Startup options become a fixed snapshot, reused by individual plugin restarts. `start --config FILE` and `run --config FILE` remain available but cannot be mixed with other startup settings. See [configuration](configuration.md) and the relevant plugin reference for business fields.

Launch a process with explicit arguments:

```sh
log-print start --input-program source=python3 \
  --list-text source.args=-u --list-text source.args=app.py \
  --text source.env.FLAG=false
```

Transform before archiving (the parent directory must exist and the target file must not):

```sh
log-print start --input-file source=./application.log \
  --output-transform numbered --set numbered.number=true \
  --output-file archive=./capture.log --read archive=numbered
```

For multiple file targets, use `--output archive=output-file`, `--read archive=source`, `--set archive.mode=create`, `--set archive.file.format=raw` and `--text archive.file.paths.source=./capture.log`. Repeat the read and path for each source. Add `--text archive.sqlite.path=./capture.sqlite` to save to both destinations. Each plugin validates its own business fields.

<span id="命名参数与高级操作"></span>

## Named parameters and advanced operations

`call` and `plugin call` accept direct `KEY=VALUE` arguments or repeated `--arg KEY=VALUE`. `true`, `false`, `null` and numbers are typed automatically; other values remain strings. `--text KEY=VALUE` forces strings; `--list KEY=VALUE` appends typed values; `--list-text KEY=VALUE` appends strings; `--empty-list KEY` supplies an empty list. Dots set nested fields. Duplicate scalars and conflicting fields fail.

```sh
log-print describe STREAM_UUID 'Build logs'
log-print call stream.describe stream=STREAM_UUID --text 'description=Build logs'
log-print plugin call archive status.get
```

Changing a description does not change stream identity or content. `--json` has been removed; use dotted fields for objects and repeated arguments for lists.

`read` starts from the oldest retained record each time and has no persistent history cursor. Use Output/SDK subscriptions for continuous consumption. The old `--from`, `--epoch`, `config set` and `session` commands/options have been removed.

State files contain management credentials; do not commit them. Starting with an occupied state file fails. Do not delete an active instance's file to bypass this check. Startup configuration changes require a main-program restart; there is no reload or hot configuration. Workbench display state is independently editable through the commands below.

Stop commands return cleanup results, such as `success: yes`, `forced: no`. A failed or forcefully terminated plugin makes the CLI exit nonzero. Restart does not create a new process after a failed stop. A Unix plugin killed by SIGKILL cannot run internal cleanup; source-process cleanup and complete persistence are then unconfirmed.

<span id="webui-命令"></span>

## WebUI commands

`--output-webui WEB` declares an all-stream, read-only WebUI. `--webui-archive WEB=DIRECTORY` creates a companion all-stream SQLite archive; `--webui-history WEB=ARCHIVE_PLUGIN` binds a SQLite output-file configured in the same instance. These archive options are mutually exclusive and never overwrite existing files.

```text
log-print webui WEB url|streams|capabilities
log-print webui WEB page list|get|create|set|clone|delete|select
log-print webui WEB panel add|get|set|clone|remove
log-print webui WEB layout set --place PANEL=LEFT,TOP,WIDTH,HEIGHT
log-print webui WEB series add|set|remove
log-print webui WEB history read|search|context|curve
log-print webui WEB query get|cancel
```

Address objects with `--page`, `--panel`, `--series` and `--query`; all settings use named options. Page/panel/series commands update backend state and synchronize browsers. `--revision` prevents overwriting concurrent changes. History commands return a task ID with fixed read boundaries. Page through it with `query get --query UUID --offset 0 --limit 200`. `history context` accepts `--seq`, `--byte-offset`, `--before` and `--after`. Curves extract a named regex capture group `value` or a JSON path supplied through `--field`. Time ranges use Unix nanoseconds. Use `--help` at each command level for all fields and see the [WebUI examples](../plugins/output-webui.md).

Core `read.range` takes stream, epoch, from, end and limit. It reads only retained memory within that epoch and returns actual oldest/head/end/next/uncovered_before coverage. Existing `read` behavior is unchanged.

<span id="终端工作台"></span>

## Terminal workbench

`--output-tui TERM`, `--tui-archive TERM=DIRECTORY` and `--tui-history TERM=ARCHIVE_PLUGIN` mirror the WebUI options; the last two are mutually exclusive. `log-print tui TERM` offers the same controls as `webui`, plus `attach [--snapshot] [--width 120] [--height 36]`. Attach can also connect to a WebUI ID. Exiting the view leaves collection running. See [output-tui](../plugins/output-tui.md).
