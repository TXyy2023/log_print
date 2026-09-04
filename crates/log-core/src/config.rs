//! Core 启动配置：插件清单、缓冲大小、QoS 阈值。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    pub ring_capacity: usize,
    pub slow_consumer_threshold: usize,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            ring_capacity: 8192,
            slow_consumer_threshold: 6144,
        }
    }
}
