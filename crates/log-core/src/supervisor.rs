//! 插件 Supervisor：spawn 子进程 + 心跳超时 + crash 拉起 + 优雅 kill。
//!
//! 单个插件 panic/OOM 只重启该插件，Core 永不宕机。

/// TODO: tokio::process + heartbeat watcher。
pub struct Supervisor;
