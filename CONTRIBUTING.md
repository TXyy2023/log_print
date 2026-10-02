# Contributing

Install Rust 1.92+, Python 3.12+, and Node.js 24/npm. Build with `cargo build --workspace --locked`, then run `python3 quality/run.py`. Historical MVP results do not establish current acceptance. See [validation](quality/README.md) for the cross-platform process, terminal and browser suites.

Keep plugin-specific business logic in plugins. Core handles stream identity, authorization and bounded memory. WebUI and TUI share the `log-view` display/history engine; serial input remains outside the active workspace. Third-party plugins can use the Rust SDK or the protocol without changing Core.

Explain each change's trigger, resulting behavior, validation and uncovered limits. For protocol, storage or identity changes, cover affected cursor, idempotency, disconnect, gap and cancellation paths. Small prose corrections do not need tests that merely match their wording.

## Documentation languages

English is the default. Update `README.md` and `README.zh-CN.md` together when public capabilities change. Maintain paired manual pages in `doc/public/` and `doc/public/zh/`, with shared assets and localized navigation. Follow [site maintenance](doc/site/README.md) for locale, search, link and public-isolation checks. Both the root path and GitHub Pages base must pass.

## Issues and changes

Provide the version/commit, OS and toolchain, sanitized configuration, minimal input, reproduction commands, expected/actual results and status diagnostics. Remove instance tokens, real device credentials and private logs; prefer synthetic data with the same structure.

Review diffs and lockfiles and run applicable checks before submitting. Runtime source, official plugins, protocol examples, public documentation and site tooling are tracked. Internal notes, user plans, captured data, machine state, archives and temporary measurements remain ignored. Only include small sanitized measurement samples with reproducible commands. Incompatible protocol changes require a protocol version change and explicit rejection of old connections.
