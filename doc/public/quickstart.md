<span id="快速开始"></span>

# Quick start

[Build the workspace](installation.md), then run these commands from the repository root. They create a synthetic input file and a dedicated instance.

```sh
python3 -c "from pathlib import Path; Path('example.log').touch()"
./target/release/log-print --state .log-print/quickstart.json start --input-file source=./example.log \
  --output-raw screen --no-autostart screen
./target/release/log-print --state .log-print/quickstart.json streams
python3 -c "open('example.log','ab').write(b'temperature=23.5\n')"
```

`start` and `streams` return the UUID assigned by Core, its description, owner and buffer range. Replace `STREAM_UUID` below with that actual UUID:

```sh
./target/release/log-print --state .log-print/quickstart.json read STREAM_UUID --wait-ms 1000
./target/release/log-print --state .log-print/quickstart.json plugin start screen --stream STREAM_UUID
./target/release/log-print --state .log-print/quickstart.json plugin stop screen
./target/release/log-print --state .log-print/quickstart.json stop
```

Results are text and tables; no JSON authoring or parsing is required. Add `--raw` to `read` for exact payload bytes. A background `output-raw` writes to the stdout log reported by `start`. For output directly in your terminal, use `run --input-file source=./example.log --output-raw screen` in a separate instance.

`read` is a snapshot of the currently retained buffer. An Output subscription keeps waiting for new data after reaching the end. See the [CLI reference](reference/cli.md) for named options; existing JSON files still work with `--config FILE`.

Core does not persist logs. Retained records may no longer include the beginning of the source, and buffer eviction does not wait for slow Outputs. For persistence, see [Display and archive](guides/archive.md).
