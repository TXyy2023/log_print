//! 极简 Core：只做路由 / 调度 / 背压 / Supervisor，不碰硬件。
//!
//! 纯库，可进 Linux 测试容器跑，无 macOS/串口依赖。

pub mod bus;
pub mod config;
pub mod framer;
pub mod qos;
pub mod ringbuffer;
pub mod supervisor;
