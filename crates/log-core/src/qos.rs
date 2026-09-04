//! QoS 降级：慢消费者跳指针 + 注入 [dropped N]，绝不阻塞串口 Loop。
//! 高频心跳做语义折叠，Agent 接口做 max_bytes/max_tokens 守卫。

/// TODO: Lossy QoS 策略。
pub struct Qos;
