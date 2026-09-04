//! input-mock：测试容器专用：发可复现假帧，无硬件依赖
//! 独立进程插件，通过 log-plugin-sdk 与 Core 全双工 IPC。

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("input-mock starting (stub)");
    // TODO: Hello 握手 -> 上报 Frame / 接收 Control
    Ok(())
}
