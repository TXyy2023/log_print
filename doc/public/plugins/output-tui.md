<span id="output-tui-终端工作台与历史上下文"></span>

# output-tui: terminal workbench and history

`output-tui` is a local Rust Output rendered with Ratatui 0.30.2 and Crossterm 0.29.0. It shares the `log-view` configuration, stream cache, line assembly, numeric extraction, history and revision engine with [output-webui](output-webui.md). Runtime needs neither Node.js nor a browser.

<span id="启动与连接"></span>

## Start and attach

```sh
# Default: live cache and Core's currently retained memory only
log-print start --input-file source=./app.log --output-tui term
log-print tui term attach

# A new instance can create a companion all-stream archive for this run
log-print start --input-file source=./app.log --output-tui term \
  --tui-archive term=./capture

# Or bind a configured SQLite output-file in the same instance
# This is mutually exclusive with --tui-archive
log-print start --input-file source=./app.log \
  --output-sqlite archive=./capture/run.sqlite \
  --output-tui term --tui-history term=archive
```

The supervisor starts background collection and a loopback API; `attach` starts a terminal view connected to it. `q` or Ctrl-C exits only that view. Collection, other terminals and background queries continue. Stop the instance with `log-print stop`. Exiting the terminal view restores the cursor, mouse capture and original terminal mode.

```sh
# Share an existing WebUI backend with browsers
log-print tui web attach

# Automation, pipes or non-TTY use: one plain-text frame, without ANSI
log-print tui term attach --snapshot --width 140 --height 36
```

Separately launched WebUI and TUI plugins have separate Page databases. Attach to an existing WebUI to share its Pages. Only one backend opens a Page database; multiple views connect to that backend rather than opening SQLite themselves.

<span id="页面与-cli"></span>

## Pages and CLI

Replace `webui` with `tui` in all workbench commands: `url`, `streams`, `capabilities`, `page`, `panel`, `layout`, `series`, `history` and `query`. `plugin call` still exposes the same control methods.

```sh
log-print tui term page create --name monitor --title 'Runtime monitor' --theme dark
log-print tui term page select --page monitor
log-print tui term page set --sidebar-open false --view-x 0 --view-y 0
log-print tui term panel add --kind log --title Logs --stream source \
  --left 0 --top 0 --panel-width 640 --panel-height 320 --column text
log-print tui term panel add --kind curve --title Temperature \
  --left 664 --top 0 --panel-width 400 --panel-height 320
log-print tui term series add --panel CURVE_UUID --name Temperature \
  --regex 'temperature=(?P<value>[0-9.]+)' --color '#5aa9fa'
log-print tui term history search --stream source --regex ERROR
log-print tui term query get --query QUERY_UUID --offset 0 --limit 200
```

Replace placeholders with returned UUIDs. In the terminal, `:` opens the same named-parameter command palette. Enter commands such as `panel set --format hex` or `series add --name Voltage --field voltage`; omitted Page/panel targets use the current selection. Commands are parsed as arguments, never executed by a shell. Editing captures the starting revision, and conflicts retain the backend's newer state.

| Action | Keys |
|---|---|
| Page list; create; clone; delete | `p`, then `n` / `c` / `d` in the list |
| All sources and identity; add a log | `s`, then `i` / Enter in the list |
| Add log / curve; select panel | `a` / `c`; Tab / Shift-Tab |
| Panel / Page properties; full command | `e` / `E`; `:` |
| Curve definitions and legend selection | `y`, then `a` / `e` / `d` / Space |
| Text / regex filter | `/` / Ctrl-R |
| Pause / follow; text / hex; metadata | Space / `f`; `t`; `i` |
| History / live / full-range search | `h` / `l` / Ctrl-F |
| Previous / next history page | `[` / `]` or PgUp / PgDn |
| Select record; context; full metadata | Up/Down; Enter; `I` |
| Coverage, gaps and query status | `o` |
| Move / resize a panel | `m` / `r`, then arrows; Enter saves, Esc cancels |
| Mouse move / resize | Drag the title / bottom-right corner |
| Canvas pan / zoom / fit all | Alt-arrows; `+` / `-` / `0` |
| Curve time zoom / legend | Ctrl-`+` / Ctrl-`-`; `g` |
| Source sidebar / minimap / position lock | `b` / `z` / `L` |
| Help / exit this terminal view | `?` / `q` or Ctrl-C |

<span id="显示映射与持久性"></span>

## Display mapping and persistence

The backend persists Page names, themes, ordering, bindings, freeform layout, filters, columns, curves, history position, pause and selection. The default database directory is `.tui/` under the instance state directory. JSON fields match WebUI: `state_path`, `listen`, `archive_dir`, `history_plugin`. When stream UUID/epoch changes, bindings resolve again by owner/alias; archives from an old run are not automatically treated as current history.

Canvas coordinates keep WebUI pixel units. At 100%, the mapping is **8 px per column and 16 px per row**. Panels support free positioning, resizing, stacking, hiding, locking, panning and zooming. Legacy 12-column GridStack layouts also render. Terminals smaller than 40×12 show a resize prompt. Logs use bounded tables; curves use Braille characters and break across gaps.

Physical font size belongs to the terminal application, and per-panel pixel font sizes or line widths cannot be reproduced exactly. Those settings remain editable and persistent through CLI and take effect in browsers. Row and column dimensions map to the character grid. Left/right arrows scroll columns horizontally. Exact column widths, sorting, legend and additional fields can be edited through the command palette. Long commands support Unicode, arrows, Home/End and safe paste.

<span id="历史、资源与故障"></span>

## History, resources and failures

Like WebUI, history defaults to 200 rows per page, curves to at most 2000 points retaining extrema and gaps, and two concurrent cancellable scans. Queries fix archive commit and Core read boundaries, deduplicate by stream/epoch/seq and preserve byte offsets for context across Record boundaries. Without an archive, only memory coverage is reported.

`o` shows actual coverage, uncommitted data, gaps and archive write failures. On backend disconnect, the last frame remains with DISCONNECTED status; the view reconnects after recovery. Display cache is limited to 64 MiB, individual HTTP responses to 4 MiB, and concurrent panel reads to four, refreshing every 500 ms. Backend pause freezes display while collection continues.

<span id="验证入口"></span>

## Validation

```sh
cargo run --locked -p log-print-quality --
# TUI alone: first build binaries and the test PTY driver
cargo build --workspace --bins --examples --locked
cargo run --locked -p log-print-quality -- suite tui-v2
```

Linux/macOS use real PTYs; Windows uses ConPTY. Tests cover keys, Chinese input, mouse dragging, resize, concurrent revisions, two views, cleanup, disconnect, non-TTY snapshots and history. The same ten backend process scenarios run against both WebUI and TUI. The three-OS GitHub matrix also runs real Chromium checks for two synchronized browser views, Vue Flow/GridStack, AG Grid, ECharts and layout restoration, saving screenshots and traces.
