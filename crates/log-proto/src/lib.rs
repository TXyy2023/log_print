//! 唯一 IPC 契约层：Frame / Control / Hello / Health，v1 先 JSON。
//!
//! 所有插件和 Core 都只能依赖本 crate 做通信，禁止插件之间直连。

pub mod control;
pub mod frame;
pub mod hello;

pub use control::Control;
pub use frame::Frame;
pub use hello::{Health, Hello};

/// 协议版本，Breaking change 时整体 +1。
pub const PROTO_VERSION: u32 = 1;
