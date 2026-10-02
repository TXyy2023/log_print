<span id="输出与归档"></span>

# Display and archive

Use `output-raw` for terminal output and `output-file` for files or databases. Core retains only bounded rolling memory, not permanent archive history.

<span id="创建新归档"></span>

## Create a new archive

The repository includes three complete configurations. Each reads a sample file in `input-file` static mode.

| Target | Configuration |
| --- | --- |
| Raw file | `project/plugins/outputs/output-file/examples/file.json` |
| SQLite | `project/plugins/outputs/output-file/examples/sqlite.json` |
| File and SQLite | `project/plugins/outputs/output-file/examples/both.json` |

Run from the repository root with a fresh, previously unused destination:

```sh
mkdir -p capture/both
./target/release/log-print --state capture/both-state.json start --config project/plugins/outputs/output-file/examples/both.json
./target/release/log-print --state capture/both-state.json streams
./target/release/log-print --state capture/both-state.json plugin call archive status.get
```

This writes `capture/both/replay.raw` and `capture/both/records.sqlite`, plus indexes, checkpoints and locks. Existing targets cause an explicit failure. Preserve old files and choose a different directory for a new capture.

<span id="确认提交再停止"></span>

## Confirm commits before stopping

First check the source report for EOF. Record its stream UUID, epoch and head. Check that the archive's `common[UUID].next` reaches `head + 1` in the same epoch, with no gaps or errors. This verifies the current snapshot, not guaranteed delivery under every workload.

```sh
./target/release/log-print --state capture/both-state.json status
./target/release/log-print --state capture/both-state.json plugin call archive shutdown
python3 -c "from pathlib import Path; assert Path('project/plugins/outputs/output-file/examples/source.log').read_bytes() == Path('capture/both/replay.raw').read_bytes(); print('bytes match')"
./target/release/log-print --state capture/both-state.json stop
```

Archive shutdown reports completion only after draining accepted records and committing. An empty queue, source EOF or receipt of a stop request cannot replace that confirmation. This byte comparison is not a power-loss durability certification.

<span id="保存自己的流"></span>

## Archive your streams

Declare output-file with `role:"output"`, authorized `reads`, target paths and `mode:"create"`. JSONL preserves full Records; raw concatenates payload bytes; SQLite preserves metadata and binary content.

An Output started later can bind to a running stream with `plugin start <plugin> --stream <actual UUID>`. Find UUIDs with `streams`. Its first subscription includes only retained memory; already overwritten history cannot be restored. The live plugin does not support resume. See [output-file](../plugins/output-file.md).

For searchable context in a browser or terminal, opt into a companion archive at startup with `--webui-archive` or `--tui-archive`. See the [WebUI](../plugins/output-webui.md) and [TUI](../plugins/output-tui.md) guides for archive coverage and query boundaries.
