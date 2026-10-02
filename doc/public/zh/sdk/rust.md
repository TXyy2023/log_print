# Rust SDK 开发指南 {#rust-sdk}

适用于 main / 0.1.3 和 `log-print/2`。`log-plugin-sdk` 是编译进插件进程的 Tokio 异步 Rust 库，不负责启动 Core。各语言交付状态见 [SDK 总览](index.md)。

## 构建与首次发布 {#example}

在插件项目中以路径依赖使用仓库 crate；本文不假定已有 crates.io 发行版：

```toml
[dependencies]
log-plugin-sdk = { path = "/absolute/log_print/project/crates/log-plugin-sdk" }
anyhow = "1"
serde_json = "1"
tokio = { version = "1", features = ["full"] }
```

执行 `cargo new sdk-example` 创建二进制项目，将上述依赖加入其 `Cargo.toml`，把以下代码保存为 `src/main.rs`：

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

在新项目中执行 `cargo build`，将其 `target/debug/sdk-example` 的绝对路径（Windows 为 `sdk-example.exe`）填入下面配置，保存为 `sdk.json`：

```json
{"core":{"transport":"tcp"},"plugins":[
  {"id":"source","role":"input","bin":"/absolute/sdk-example/target/debug/sdk-example","streams":[{"id":"samples"}]}
]}
```

构建主工作区二进制后，执行 `log-print --state ./sdk-state.json start --config ./sdk.json`，再用 `log-print --state ./sdk-state.json resolve samples` 取得 `id` 字段的 UUID，通过 `log-print --state ./sdk-state.json read STREAM_UUID --raw` 读取，最后执行 `log-print --state ./sdk-state.json stop`。若未加入 PATH，使用构建产物 `target/debug/log-print` 的路径。该示例发布一次后处理通用控制，并等待协作停止。

## 连接与配置 {#connections}

| 入口 | 行为 |
| --- | --- |
| `connect_env()` | 返回 `(Client, Receiver<Event>, Receiver<Control>)`，调用方自行处理控制 |
| `Client::connect(address, plugin, token)` | 显式 TCP 连接，配置为空 |
| `Client::connect_with_config(...)` | 显式 TCP 连接及 JSON 配置 |
| `Client::connect_with_transport(..., kind)` | 选择与 Core 一致的 TCP 或 UDP |
| `connect(validate)` | 注册、校验配置，返回 `Context` 并启动通用控制任务 |

环境注册必需 `LOG_PRINT_CORE`、`LOG_PRINT_PLUGIN`、`LOG_PRINT_TOKEN`。`LOG_PRINT_CONFIG` 默认 `{}`；`LOG_PRINT_TRANSPORT` 默认 `tcp`，也接受 `udp`。注册有 10 秒截止时间。Core 地址不同于 supervisor 的管理地址。

`Context` 暴露 `client`、`events`、`shutdown` 和校验后的 `config`。停止后调用 `shutdown_result()` 检查控制故障。`Client::config()` 保留原始配置，可能与校验后的 `Context.config` 不同。

## Client API {#api}

| API | 返回或用途 |
| --- | --- |
| `config()`、`transport()` | 启动 JSON 配置与传输方式 |
| `stream_id()`、`input_stream()`、`own_stream()` | 相同的可选自有流 UUID |
| `read_streams()` | 注册时可读流 UUID 快照 |
| `streams()` | 分页汇总可见目录，返回去重后的 JSON 数组 |
| `stream(id)`、`resolve_stream(alias_or_uuid)` | 查询流元数据；解析流引用为 UUID |
| `create_stream(description, parents)` | 请求创建获准的流，使用返回的 UUID |
| `read_range(stream, epoch, from, end, limit)` | 读取保留内存中的记录及覆盖信息 |
| `publish(...)`、`publish_with_source_seq(...)`、`publish_tagged(...)` | 发布字节和来源信息 |
| `subscribe(stream)`、`unsubscribe(stream)` | 建立或移除单流订阅 |
| `request(op, args)` | 返回 JSON 的通用授权 RPC |
| `reply_control(call_id, result, error)` | 回复收到的控制请求 |

`create_stream()` 不更新已保存的自有流 UUID。目录查询不是原子快照。范围读取要求 `from > 0`、`limit` 为 1..64；结果包含 `records`、`next`、`oldest`、`head`、`uncovered_before`，需按 `next` 翻页并检查覆盖情况，不会查询归档数据库。

发布参数为 `stream: &str`、`key: &str`、`payload: Vec<u8>`、`source_ts_ns: Option<u64>`、`upstream: BTreeMap<String, u64>`。`publish_with_source_seq` 增加 `source_seq: Option<u64>`；`publish_tagged` 依次增加 `channel: Option<String>` 和 `source_seq`。payload 是任意字节，编码前最多 64 KiB；传输帧与 Core 配置可能施加更低限制。key 由调用方提供，不应视作 exactly-once 保证。

TCP 返回 `PublishOutcome::Accepted(Box<Record>)`，UDP 返回 `LocalSent`，都不证明持久保存。Record 包含 stream、epoch、seq、key、payload、source_ts_ns、observed_ts_ns、upstream、upstream_epochs、source_seq、channel。

## 事件、停止与错误 {#lifecycle}

`Event` 包含 `Record(Record)`、`Gap { stream, epoch, from, to, reason }` 和 `Disconnected { stream, reason }`。应持续消费接收端；所有订阅共用有界事件队列，每条流使用独立事件连接，从最早保留记录开始。没有自动重连或游标恢复。订阅断连后需先 `unsubscribe()` 再重订阅；没有 Gap 不代表数据完整。

通用 `connect(validate)` 处理器支持 `config.get`，以 `restart_required` 拒绝 `config.patch`，并先以 `stopping:true, completed:false` 回复 `shutdown`，再发出停止信号。业务循环可等待 `stopped(&mut shutdown)`，随后检查 `shutdown_result()` 并完成业务清理。`finish(&client, &result)` 仅在错误时尝试报告失败，不报告业务成功完成。自定义控制需使用底层控制接收端。

RPC 有 30 秒截止时间。`timeout_unknown` 表示远端可能已经执行；`connection_lost` 使等待请求失败。取消不会撤销已发送请求，不自动重试。释放最后一个 Client 会终止连接及订阅任务，不会等待业务存储提交。

## 源码与验证 {#source}

参见 [Client 实现](https://github.com/TXyy2023/log_print/blob/main/project/crates/log-plugin-sdk/src/lib.rs)、[生命周期辅助](https://github.com/TXyy2023/log_print/blob/main/project/crates/log-plugin-sdk/src/lifecycle.rs)和[最小示例](https://github.com/TXyy2023/log_print/blob/main/project/crates/log-plugin-sdk/examples/minimal.rs)。执行 `cargo test -p log-plugin-sdk` 验证 SDK；真实进程验收见仓库的[质量指南](https://github.com/TXyy2023/log_print/blob/main/quality/README.md)。
