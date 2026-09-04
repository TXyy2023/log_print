//! input-log：软件日志插件：Tail上行 / 调级别+Seek下行
//! 独立进程插件，通过 log-plugin-sdk 与 Core 全双工 IPC。

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("input-log starting (stub)");
    // TODO: Hello 握手 -> 上报 Frame / 接收 Control
    Ok(())
}
