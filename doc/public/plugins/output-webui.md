# output-webui: local log viewer {#output-webui}

A single Vue interface backed by Rust, with all assets embedded for offline use. Select a source, follow or pause the display, find matching text, inspect context, import files and save original logs.

![Log viewer](../assets/webui/log-viewer.png)

Captured from the real service using explicitly marked fixture logs.

## Start {#start}

```sh
log-print start --output-webui web
log-print webui web url

log-print start --input-file source=./app.log --output-webui web --webui-archive web=./capture
```

Without archiving, only retained memory is available. `--webui-archive` creates a fresh SQLite archive. Alternatively, use `--output-sqlite archive=./capture/run.sqlite --webui-history web=archive` to bind an output-file archive configured in the same instance. These options are mutually exclusive and do not overwrite existing files.

## Read and locate {#read}

The source rail distinguishes live streams and imported files. Live mode shows up to 2,000 rows using virtualization. History pins the available archive/memory boundaries and paginates up to 200 rows per page.

- Pausing freezes display while collection and saving continue. Scrolling away from the tail exposes a return-to-latest action.
- Literal keywords highlight locally; matching-only filtering applies to the selected range. Enter/Shift Enter navigates matches in the current window or history page. Highlight-only edits do not rescan the archive.
- Select a row to copy original text or inspect its stream, record sequence, byte offset and channel. Context reads eight lines before and after the selected location, including lines outside the filter.
- The time column is collection time. Timestamps inside the payload are not silently reinterpreted. A transport Record is not necessarily one line.
- Eviction, archive failures and gaps are explicit. Displayed data does not prove persistence.

## Import and record {#import}

Upload or drop one file up to 64 MiB into a TCP supervisor instance. Each file uses an independent input-file process and Core stream, then output-file's archive writer. Bounded publication waits for durable checkpoints. Completion requires source EOF, matching byte counts and no output gaps. Complete imported history remains searchable beyond the display cache. Failed imports retain the source file.

## Record logs {#record}

The save menu offers two starting points:

| Start | Behavior |
| --- | --- |
| From now | Pin the Core head when the backend handles the action and save subsequent raw records |
| Earliest available | Merge readable archive, WebUI cache and Core history without duplicate sequences, then continue recording |

The recordings dialog shows source, progress, failures and committed files. Stop drains to a fixed head and commits before enabling `.log` download. Display filters and pause do not change saved content. Files are created under the WebUI state directory at `recordings/<UUID>/`. Files survive shutdown; the runtime recording list and imported stream catalog are not automatically rebuilt on supervisor restart. At most four recordings run concurrently, with 64 jobs/import streams per runtime.

## Keyboard and responsive layout {#keyboard}

Ctrl/Cmd F focuses search. Enter/Shift Enter navigates matches in the search box. Arrow keys, PageUp/PageDown and Home/End navigate the log grid; Enter opens details. Space outside inputs pauses/resumes. Escape closes overlays; `?` opens help. On smaller screens, sources and details become drawers, while long lines scroll inside the table. Theme and selected stream persist locally; the filter and selected row reset on refresh.

## CLI and HTTP {#cli}

```sh
log-print webui web streams
log-print webui web history read --stream source
log-print webui web history search --stream source --regex '^ERROR'
log-print webui web history context --stream source --epoch EPOCH --seq 123 --byte-offset 20 --before 8 --after 8
log-print webui web query get --query QUERY_UUID --offset 0 --limit 200
```

Use `POST /api/control` for `logs.tail` (stream/text/limit/since), `recording.start` (stream/mode=now|earliest), `recording.stop` (id), and the shared history/query methods. Upload with `POST /api/import` using a raw body and URI-encoded `X-File-Name`; download with `GET /api/recordings/<id>/download` after successful stop/commit.

The former canvas workbench frontend, chart/layout dependencies and alternate entry point have been removed. CLI/TUI still use the shared Rust query/configuration engine; browser viewing state is independent. Host/Origin checks protect writes and credentials stay in Rust.
