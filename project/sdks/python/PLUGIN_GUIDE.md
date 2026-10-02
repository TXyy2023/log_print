# Build a plugin with the Python SDK

Start with the installed library's public API, not JSONL sockets or a Rust Client
translation. Read README.md's lifecycle and delivery guarantees first.

1. Choose Input (collect and `send`), Output (`records` then act), or Transform
   (`records` then `send` with actual upstream positions).
2. Use `async with log.init() as app`. Existing async applications call their
   coroutine directly; executable scripts use one `asyncio.run(main())`.
3. Use str or exact bytes. Decode explicitly. Never invent source timestamps,
   source sequence or upstream progress. Do not catch and silently ignore SDKError.
4. Define business configuration defaults and validate them with `validate=...`;
   return the effective mapping. `app.config` is a copied startup snapshot.
5. Keep blocking I/O off the asyncio loop, with a business-level timeout.
6. For managed plugins, declare bin/args/reads/streams/parents. Reuse the host
   virtualenv interpreter with absolute paths. Never commit credentials.
7. Verify real output against synthetic expected bytes, normal stop, failed input,
   and connection loss. A successful publish is not successful storage.
8. Deliver script, configuration, installation/start/stop commands and actual
   verification output. Do not require modifications to the SDK for ordinary I/O.

Templates live in examples/. The output file example demonstrates visibility,
not crash-safe persistence. Production archive plugins must define and implement
flush/commit/failure semantics. Independent default `init()` sessions are isolated;
use a supervisor instance when separate plugins need to share streams.

Run the installed-wheel suite via `python quality/tests/python_sdk.py` from the
repository root after building Rust binaries. Extra business dependencies belong
to the plugin's package, not the SDK. No requests/numpy/device package is required
for the three synthetic examples.
