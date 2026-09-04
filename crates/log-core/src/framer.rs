//! 流式分帧：CRLF / 定长 / 超时 + UTF-8 lossy 清洗。

/// TODO: 高吞吐零拷贝分帧器。
pub struct Framer;

impl Framer {
    pub fn push(&mut self, _chunk: &[u8]) -> Vec<Vec<u8>> {
        todo!("framer push")
    }
}
