# 项目代码

所有构建命令均从仓库根目录运行。

| 目录 | 内容 |
| --- | --- |
| `crates/` | app-log-print、log-core、log-proto、log-plugin-sdk 四个核心 |
| `plugins/inputs/` | input-file、input-program 两个输入 |
| `plugins/outputs/` | output-raw、output-file、output-transform 三个输出 |
| `examples/` | 使用示例 |
| `skills/` | 项目 Agent 使用指南 |
| `workloads/` | 外部真实软件日志工作负载，区别于测试生成输入 |

`input-serial`、TUI、WebUI 暂缓且不参与 workspace。已取消的组件和旧协议测试在 `quality/archive/v1/` 保留历史，当前验收从 [quality](../quality/README.md) 运行。
