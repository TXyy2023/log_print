# 项目代码

所有构建和示例命令均从仓库根目录运行。

| 目录 | 内容 |
| --- | --- |
| `crates/` | CLI、Core、协议、Rust SDK，以及暂缓的 log-plot |
| `plugins/inputs/` | 输入插件及共享输入工具 |
| `plugins/outputs/` | 输出插件 |
| `examples/` | 使用示例；插件专用示例保留在对应插件内 |
| `skills/` | 随项目提供的 Agent 技能 |
| `workloads/` | 外部真实软件日志工作负载候选与约束，不自动加入 CI |
| `.local/archive/` | 本地历史源码归档，不参与构建和提交 |

```sh
cargo build --workspace --locked
python3 quality/run.py
```

workspace 的成员和暂缓组件以根目录 [Cargo.toml](../Cargo.toml) 为准。独立验收脚本统一位于 [quality/](../quality/README.md)，Rust 源码内单元测试保留原位。
