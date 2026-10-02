# SDK overview {#sdk-overview}

The SDK connects plugin code to Core using `log-print/2`. Core owns identities, permissions, stream UUIDs and bounded memory; plugins own capture, transformation, display and persistence.

## Choose an SDK {#choose}

| SDK | Availability | Entry point | Transport |
| --- | --- | --- | --- |
| [Rust SDK](rust.md) | Included in main / 0.1.3, crate `log-plugin-sdk` | `connect_env()` or `connect(validate)` | TCP or UDP |
| [Python SDK](python.md) | Implemented on `codex/python-sdk`, commit `6be6992`; not included in main / ver-0.1.3 | `async with log.init() as app` | TCP only |

Python branch status was checked on 2026-10-03. Install it from the source revision linked in its guide; the main checkout has no `project/sdks/python` directory. No public package-index release is assumed.

## Common workflow {#workflow}

1. Declare a plugin executable, role and stream permissions in the [instance configuration](../reference/configuration.md).
2. The supervisor starts the process and hands over its Core address, identity, token and configuration through `LOG_PRINT_*` environment variables.
3. Input plugins publish bytes to owned streams. Output plugins subscribe to permitted streams. A transform is an Output with its own derived stream and declared parents.
4. Handle shutdown cooperatively, finish business work and report its actual result.

The Rust SDK attaches to an existing Core. Python additionally provides an isolated local mode that starts an already installed Core binary and cleans up only its own instance. Separate local sessions do not share streams.

## Delivery and lifecycle {#guarantees}

- TCP acceptance means Core admitted a record; it does not confirm a consumer processed or persisted it. Rust UDP `LocalSent` confirms only local transmission.
- Subscriptions read retained records then wait for new records. Core memory may overwrite unread history; no gap event does not prove completeness.
- Range reads query retained Core memory. Persistent history belongs to [output-file](../plugins/output-file.md), not the SDK.
- No automatic reconnect, retry, durable outbox or exactly-once delivery is provided. A timeout or cancellation can leave the remote outcome unknown.
- Shutdown acknowledgement is not business completion. Configuration changes require restart.

Continue with the [Rust API and example](rust.md) or [Python installation, examples and API](python.md).
