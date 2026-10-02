# log-print Python SDK

Python 3.12+, standard library at runtime. A Python-native async library for
`log-print/2`: `init`, `send`, `records`. Protocol/identity/controls are internal.
This is the repository's SDK; no public package-index release is assumed.

## Install locally

Build the companion Core once (or use your existing compatible binary):

```sh
cargo build --locked --workspace
python -m pip install ./project/sdks/python
```

Put `log-print-core` on PATH, set `LOG_PRINT_CORE_BIN` to its absolute path, or pass
`core_binary="/absolute/path/log-print-core"` to `init`. Windows uses
`log-print-core.exe`. The Python package needs no Rust compiler at runtime.

## Send and receive with defaults

```python
import asyncio
import log_print_sdk as log

async def main():
    async with log.init() as app:
        sent = await app.send("hello\n")
        records = app.records()
        received = await anext(records)
        assert received.payload == sent.payload
        print(received.stream, received.seq, received.text())

asyncio.run(main())
```

Without a managed environment, `init()` starts an **isolated local Core**, with
separate input and output identities. Context exit closes its connections and
stops only that owned Core. Separate default init calls create separate instances,
not a shared global service. Importing the package does not connect or spawn.
The Core binary must already exist; the library never downloads it.

For independent input/output programs sharing a service, launch them as managed
plugins with the supervisor configuration below. They use the same `init()` API.
Explicit `address=`, `plugin=`, `token=` can also connect an already provisioned
identity; these three options must be supplied together. Never hard-code tokens.

## Managed plugins

```python
# input.py
import asyncio
import log_print_sdk as log

async def main():
    async with log.init() as app:
        while not app.stopping:
            await app.send("temperature=23.5\n")
            await app.sleep(1)

asyncio.run(main())
```

```python
# output.py
import asyncio
import log_print_sdk as log

async def main():
    async with log.init() as app:
        async for record in app.records():
            print(record.stream, record.seq, record.text())

asyncio.run(main())
```

`init()` reads LOG_PRINT_* handed over by the supervisor, uses its default writable
or readable streams, and handles controls and shutdown. It never owns the managed
Core. Missing/partial credentials fail rather than silently launching a new Core.
Use this configuration with `log-print --state ./python-state.json start --config
./python.json`; stop with `log-print --state ./python-state.json stop`:

```json
{
  "core": {"transport": "tcp"},
  "plugins": [
    {"id":"source", "role":"input", "bin":"/absolute/venv/bin/python",
     "args":["/absolute/input.py"], "streams":[{"id":"samples"}]},
    {"id":"sink", "role":"output", "bin":"/absolute/venv/bin/python",
     "args":["/absolute/output.py"], "reads":["samples"]}
  ]
}
```

On Windows, use the absolute `venv\\Scripts\\python.exe` path and escape JSON
backslashes. Paths passed in args are absolute, including those containing spaces.
Configuration is needed to connect independent processes and permissions, not to
manually specify low-level protocol parameters. No JSON is needed in local mode.

## API and guarantees

- `async with init(...) as app`: owns session resources; integrates with an existing
  asyncio loop. `timeout=30`, `queue_size=64`, `config={}`, `validate=callable`,
  `core_binary=...`, `core_options={...}` are optional. Core options apply only to
  local mode. A validator returns the effective configuration mapping.
- `await app.send(str_or_bytes, *, stream=None, key=None, channel=None,
  source_seq=None, source_ts_ns=None, upstream=None)` returns the full accepted
  `Record`. Strings use UTF-8; bytes remain exact; no implicit newline or splitting.
  Default keys identify this session's sends, **not** an idempotency mechanism.
- `async for record in app.records(stream="alias")`: streams retained and future
  records. Without stream, uses configured reads. Local mode reads its own source
  via a separate Output identity. Each iterator owns subscriptions; overlapping
  subscriptions fail. `await records.aclose()` releases them early; context exit
  always cleans them up. Callers must not concurrently advance the same iterator.
- `Record` includes `stream`, `epoch`, `seq`, `key`, `payload: bytes`,
  `source_ts_ns`, `observed_ts_ns`, `upstream`, `upstream_epochs`, `source_seq`,
  `channel`. `.text(encoding="utf-8", errors="strict")` explicitly decodes bytes.
- `app.config` returns a copy of the startup snapshot; `app.stream` is the owned
  UUID; `app.stopping` and `await app.sleep(seconds)` support cooperative loops.
- `await app.streams()`, `await app.read_range(stream, epoch=..., start=..., end=...,
  limit=64)`, `await app.create_stream(description, parents=[...])` and
  `await app.report(state=...)` are advanced operations. Range results retain
  Core coverage metadata and convert the records to Record objects.

TCP only. Passing UDP via the managed environment fails explicitly. Permission
checks remain in Core. A Transform is an Output with an owned derived stream and
parents; see `examples/transform.py`. Publishing returns Core acceptance, not
consumer processing or persistence. EOF is not global stream completion.

Queues are bounded. Slow readers can lose overwritten Core history; absence of
DataGapError does not prove completeness (current Core does not emit all gaps).
`records()` snapshots configured reads when it starts; read_all users must discover
new streams via `streams()` and explicitly subscribe. No automatic reconnection,
retry, durable cursor, outbox or recovery is provided.

Failures raise SDKError subclasses: ConfigurationError, PermissionDeniedError,
PayloadTooLargeError, ProtocolError, ConnectionLostError, OutcomeUnknownError,
DataGapError, StoppedError. Remote errors retain `.code`. OutcomeUnknownError means
a request may have executed; do not blindly retry. Cancellation remains Python's
CancelledError; cancellation also does not undo an operation already sent.

Initialization reports sdk_ready (connection/configuration only, not business readiness);
normal context exit reports stopped with business_complete=false for supervisor
compatibility. Business progress/completion remains the plugin's responsibility.
Shutdown acknowledges stopping, not completion; async iteration ends normally on
requested shutdown. Connection loss raises. Config updates require restart.
Exceptions leaving a context report failure type, not exception text that might
contain secrets. Blocking device or file operations should use timeouts and
`asyncio.to_thread`; blocking the event loop prevents timely shutdown.

## Develop and verify

See [TESTING.md](TESTING.md) for the CI contract and [PLUGIN_GUIDE.md](PLUGIN_GUIDE.md)
for the AI/plugin checklist. From repository root:

```sh
cargo build --locked --workspace
python quality/tests/python_sdk.py
```

This builds a wheel, installs into a clean venv, then tests real Core and Rust
plugin interoperability outside the source checkout. It does not publish a package.
