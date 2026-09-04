use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// 上行数据帧：Input -> Core -> Outputs。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub seq: u64,
    pub ts_ms: u64,
    pub source_id: String,
    pub payload: Vec<u8>,
    pub kind: FrameKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FrameKind {
    #[default]
    Data,
    Warning,
}

impl Frame {
    pub fn bytes(&self) -> Bytes {
        Bytes::copy_from_slice(&self.payload)
    }
}
