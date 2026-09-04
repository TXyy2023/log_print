use serde::{Deserialize, Serialize};

/// 下行控制指令：Core/Agent -> Inputs/Outputs，全双工的关键。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Control {
    pub target: String,
    pub op: ControlOp,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlOp {
    WriteSerial { bytes: Vec<u8> },
    SetBaud { baud: u32 },
    SetDtr { level: bool },
    SetRts { level: bool },
    SetLevel { level: String },
    Seek { offset: u64 },
    SetFilter { rule: String },
}
