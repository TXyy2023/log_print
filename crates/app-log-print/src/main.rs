//! log_print 瘦 CLI：解析参数 -> 起 Core -> spawn 插件子进程。
//!
//! 自身不做业务，只做组装和生命周期管理。

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("log_print starting (stub)");
    // TODO: 加载 CoreConfig -> Bus + Supervisor -> spawn inputs/outputs
    Ok(())
}
