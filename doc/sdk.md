# Rust SDK 接入

依赖 `log-plugin-sdk` 和 `log-proto`。SDK 使用 Tokio；官方插件提供最小可复用的实际示例。协议是接入标准，第三方可独立按规范实现，不要求使用 Rust；其他语言 SDK 留待后续。

```rust,no_run
use std::collections::BTreeMap;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client, _events, _controls) = log_plugin_sdk::connect_env().await?;
    client.publish_retained("example", "run-unique:1", b"hello\0world".to_vec(),
        None, BTreeMap::new()).await?;
    client.request("report", serde_json::json!({"state":"completed"})).await?;
    Ok(())
}
```

可构建例子在 `crates/log-plugin-sdk/examples/minimal.rs`；正式插件应为每次源运行生成新身份，并保留每块 key 直到确认。长期源需并行处理 controls 中的 shutdown/config 方法，不能忽略停止请求。

主程序注入 LOG_PRINT_CORE / LOG_PRINT_PLUGIN / LOG_PRINT_TOKEN / LOG_PRINT_CONFIG。不要手工硬编码令牌；不要将这些变量传给被采集程序。业务配置经 `client.config()` 读取，各插件需要校验未知字段、范围和动态生效方式。

`connect_env()` 返回可克隆 Client、事件 Receiver、控制 Receiver。常用方法：

| 方法 | 用途 |
|---|---|
| request(op,args) | 有截止时间的通用请求，结构化 Fault 可 downcast |
| publish / publish_retained | 发布字节、key、源时间及父进度；后者仅在保存阻塞/未知提交时保留当前块等待手动恢复 |
| subscribe / subscribe_epoch | 单流独立事件连接；多流重复调用；epoch版本验证 |
| unsubscribe | 关闭本地订阅任务及连接 |
| reply_control | 回复 Core 路由来的 call_id，明确成功或 Fault |

事件为 Record / Gap / Disconnected。Disconnected 不含可证明的丢失条数，消费端应更新状态或停止，不能继续显示数据完整。派生处理用 Record.stream/epoch/seq 保持各父进度，不修改原始记录。

SDK 的 RPC 工作队列与事件队列有界。整帧由独立发送任务完成，取消请求的等候不截断帧；取消或超时仍可能已执行，保存发布通过相同 key 恢复，非幂等控制不要自动重复。控制忙返回 control_busy，连接失败清除 pending 并关闭控制接收端。Drop 所有 Client 关闭后台连接；运行中临时克隆不改变身份。

插件模板：在 `plugins/inputs/<name>/` 或 `plugins/outputs/<name>/` 新增 Cargo.toml 和 src/main.rs，使用 workspace 的版本/依赖；声明主程序配置中的 id/bin/reads/streams。物理目录不限制双向发布订阅能力。运行 `cargo check -p <name>`，再用真实 Core 子进程验收字节、退出和错误路径。
