//! 全双工路由表：source_id -> consumers，Control 按 target 反向路由。

use log_proto::{Control, Frame};

/// TODO: tokio::sync::broadcast / mpsc + 每消费者独立指针。
pub struct Bus;

impl Bus {
    pub fn new() -> Self {
        Self
    }

    pub fn publish(&self, _frame: Frame) {
        todo!("bus publish")
    }

    pub fn dispatch_control(&self, _ctl: Control) {
        todo!("bus dispatch_control")
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}
