use serde::{Deserialize, Serialize};

/// 插件启动握手。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub plugin_id: String,
    pub version: u32,
    pub capabilities: Vec<String>,
}

/// 心跳，供 Supervisor 做存活与 lag 判断。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub plugin_id: String,
    pub lag_frames: u64,
}
