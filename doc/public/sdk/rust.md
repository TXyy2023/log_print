# Rust SDK {#rust-sdk}

Applies to main / 0.1.3 and `log-print/2`. `log-plugin-sdk` is a Tokio-based Rust library compiled into the plugin process. It does not start Core. See [SDK overview](index.md) for language availability.

## Build and first publication {#example}

Use the repository crate as a path dependency in your plugin project; this guide does not assume a crates.io release:

```toml
[dependencies]
log-plugin-sdk = { path = "/absolute/log_print/project/crates/log-plugin-sdk" }
anyhow = "1"
serde_json = "1"
tokio = { version = "1", features = ["full"] }
```

Create a binary project with `cargo new sdk-example`, add the dependencies above to its `Cargo.toml`, and save this as `src/main.rs`:

```rust
use std::collections::BTreeMap;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut context = log_plugin_sdk::connect(|config| Ok(config.clone())).await?;
    let client = &context.client;
    let stream = client.stream_id()
        .ok_or_else(|| anyhow::anyhow!("declare one input stream"))?;
    let outcome = client.publish_tagged(
        stream, "example:1", b"hello\0world".to_vec(), None,
        BTreeMap::new(), None, Some(1),
    ).await?;
    eprintln!("{outcome:?}");
    client.request("report", serde_json::json!({"state":"ready"})).await?;
    log_plugin_sdk::stopped(&mut context.shutdown).await;
    context.shutdown_result()
}
```

Build in the new project with `cargo build`. Put the absolute path to its `target/debug/sdk-example` (Windows: `sdk-example.exe`) in this configuration and save it as `sdk.json`:

```json
{"core":{"transport":"tcp"},"plugins":[
  {"id":"source","role":"input","bin":"/absolute/sdk-example/target/debug/sdk-example","streams":[{"id":"samples"}]}
]}
```

With the main workspace binaries built, run `log-print --state ./sdk-state.json start --config ./sdk.json`, resolve the alias with `log-print --state ./sdk-state.json resolve samples`, read the UUID in the returned `id` field with `log-print --state ./sdk-state.json read STREAM_UUID --raw`, then run `log-print --state ./sdk-state.json stop`. Use the built `target/debug/log-print` path if it is not on PATH. The sample publishes once, handles common controls and waits for cooperative shutdown.

## Connections and configuration {#connections}

| Entry | Behavior |
| --- | --- |
| `connect_env()` | Returns `(Client, Receiver<Event>, Receiver<Control>)`; caller handles controls |
| `Client::connect(address, plugin, token)` | Explicit TCP connection, empty configuration |
| `Client::connect_with_config(...)` | Explicit TCP connection with JSON configuration |
| `Client::connect_with_transport(..., kind)` | Selects TCP or UDP, matching Core |
| `connect(validate)` | Registers, validates configuration, returns `Context` and starts common control handling |

Environment registration requires `LOG_PRINT_CORE`, `LOG_PRINT_PLUGIN` and `LOG_PRINT_TOKEN`. `LOG_PRINT_CONFIG` defaults to `{}`; `LOG_PRINT_TRANSPORT` defaults to `tcp` and accepts `udp`. Registration has a 10-second deadline. The Core endpoint is distinct from the supervisor's management endpoint.

`Context` exposes `client`, `events`, `shutdown` and validated `config`. Call `shutdown_result()` after stopping to detect control failures. `Client::config()` retains the original configuration, which can differ from validated `Context.config`.

## Client API {#api}

| API | Result or purpose |
| --- | --- |
| `config()`, `transport()` | Startup JSON configuration and transport |
| `stream_id()`, `input_stream()`, `own_stream()` | Same optional owned UUID |
| `read_streams()` | Registration-time readable UUID snapshot |
| `streams()` | Paginates the visible catalog and returns a deduplicated JSON array |
| `stream(id)`, `resolve_stream(alias_or_uuid)` | Stream metadata; resolve a reference to UUID |
| `create_stream(description, parents)` | Request an authorized stream; use the returned UUID |
| `read_range(stream, epoch, from, end, limit)` | Retained-memory records and coverage metadata |
| `publish(...)`, `publish_with_source_seq(...)`, `publish_tagged(...)` | Publish bytes and source metadata |
| `subscribe(stream)`, `unsubscribe(stream)` | Start or remove a per-stream subscription |
| `request(op, args)` | Generic authorized RPC returning JSON |
| `reply_control(call_id, result, error)` | Reply to a received control request |

`create_stream()` does not update the stored owned UUID. Catalog queries are not atomic snapshots. Range reads require `from > 0` and `limit` in 1..64; results include `records`, `next`, `oldest`, `head` and `uncovered_before`. Follow `next` for pagination and inspect coverage. They do not query archive databases.

Publication takes `stream: &str`, `key: &str`, `payload: Vec<u8>`, `source_ts_ns: Option<u64>` and `upstream: BTreeMap<String, u64>`. `publish_with_source_seq` adds `source_seq: Option<u64>`; `publish_tagged` adds `channel: Option<String>` followed by `source_seq`. Payloads are arbitrary bytes, at most 64 KiB before encoding; transport frame and Core limits can be lower. The caller supplies keys; do not treat them as an exactly-once guarantee.

TCP returns `PublishOutcome::Accepted(Box<Record>)`; UDP returns `LocalSent`. Neither proves persistence. Record fields include stream, epoch, seq, key, payload, source_ts_ns, observed_ts_ns, upstream, upstream_epochs, source_seq and channel.

## Events, shutdown and errors {#lifecycle}

`Event` has `Record(Record)`, `Gap { stream, epoch, from, to, reason }` and `Disconnected { stream, reason }`. Consume the receiver continuously. All subscriptions share a bounded event queue; each stream has a separate event connection starting at the oldest retained record. No automatic reconnect or cursor resume is provided. After a subscription disconnects, call `unsubscribe()` before subscribing again. No Gap does not establish completeness.

The common `connect(validate)` handler supports `config.get`, rejects `config.patch` with `restart_required`, and acknowledges `shutdown` with `stopping:true, completed:false` before signaling stop. Use `stopped(&mut shutdown)` in the business loop, then check `shutdown_result()` and finish business cleanup. `finish(&client, &result)` attempts a failure report on error; it does not report successful business completion. Custom controls require the lower-level control receiver.

RPC requests have a 30-second deadline. A `timeout_unknown` fault means the remote operation may have executed; `connection_lost` fails pending requests. Cancellation does not undo an already sent request. No automatic retry is performed. Dropping the last Client cancels connection and subscription tasks, not business storage commits.

## Source and validation {#source}

See [Client implementation](https://github.com/TXyy2023/log_print/blob/main/project/crates/log-plugin-sdk/src/lib.rs), [lifecycle helpers](https://github.com/TXyy2023/log_print/blob/main/project/crates/log-plugin-sdk/src/lifecycle.rs) and [minimal example](https://github.com/TXyy2023/log_print/blob/main/project/crates/log-plugin-sdk/examples/minimal.rs). Run `cargo test -p log-plugin-sdk` for SDK tests. Process-level validation is documented in the repository's [quality guide](https://github.com/TXyy2023/log_print/blob/main/quality/README.md).
