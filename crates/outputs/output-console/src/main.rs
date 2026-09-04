//! output-console：控制台高亮输出插件
//! 独立进程插件，通过 log-plugin-sdk 与 Core 全双工 IPC。

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("output-console starting (stub)");
    // TODO: Hello 握手 -> 上报 Frame / 接收 Control
    Ok(())
}
