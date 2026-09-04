//! 插件 SDK：帮 Input/Output 子进程快速接入（transport + 握手 + 心跳 + 重连）。
//!
//! 插件作者只需实现 on_frame / on_control 两个回调。

pub mod heartbeat;
pub mod retry;
pub mod transport;

pub use log_proto::{Control, Frame, Hello};

/// 插件入口 trait（MVP 草稿）。
#[allow(async_fn_in_trait)]
pub trait Plugin {
    async fn on_frame(&mut self, frame: Frame);
    async fn on_control(&mut self, ctl: Control);
}
