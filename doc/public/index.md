<span id="log-print-使用手册"></span>

# log_print manual

This manual follows the current source tree, version **0.1.3 / log-print/2**. Two input plugins feed Core's memory buffers; five output plugins display, archive or transform the streams. Build from the same source revision as this manual: `ver-0.1.3` includes the CLI, WebUI and TUI updates introduced after `ver-0.1.2`.

<span id="开始使用"></span>

## Get started

- [Install and build](installation.md)
- [Quick start](quickstart.md)
- [Streams, transport and persistence](concepts.md)

<span id="按任务选择"></span>

## Choose a task

| Task | Guide |
| --- | --- |
| Launch a process or attach to an existing tmux pane | [Capture program output](guides/program.md) |
| Follow a growing log file | [Read a log file](guides/file.md) |
| Read a static file as quickly as possible | [Import a static file](guides/replay.md) |
| Find stream IDs, read logs and manage an instance | [Read and manage](guides/read.md) |
| Add numbering, timestamps or bounded source-sequence reordering | [Transform logs](guides/transform.md) |
| Display logs or save raw files, JSONL and SQLite | [Display and archive](guides/archive.md) |
| Arrange persistent log and chart panels in a browser | [WebUI workbench](plugins/output-webui.md) |
| Use the same display and history engine in a terminal | [TUI workbench](plugins/output-tui.md) |
| Check shutdown and persistence results | [Integrity and migration](guides/recovery.md) |

See the [CLI reference](reference/cli.md), [configuration reference](reference/configuration.md) and [troubleshooting guide](troubleshooting.md). Serial input is outside the current workspace. The separate input-replay, io-plugin-util and log-plot components have been retired.

English is the default documentation language. Use the language menu for the equivalent Simplified Chinese page, or open the [Chinese manual](zh/index.md).
