# log_print 使用手册

本手册对应 **0.1.2 / log-print/2**。日志从两个输入插件进入 Core 内存缓冲，再由三个输出插件分别显示、保存或加工。

## 开始使用

- [安装与构建](installation.md)
- [快速开始](quickstart.md)
- [流、传输与保存的边界](concepts.md)

## 按任务选择

| 任务 | 指南 |
| --- | --- |
| 通过 CLI 启动程序，或接入已有 tmux 窗格 | [采集程序输出](guides/program.md) |
| 持续跟随日志文件 | [文件输入](guides/file.md) |
| 尽快读取静态文件 | [静态文件导入](guides/replay.md) |
| 查看流 ID、读取与管理实例 | [读取与管理](guides/read.md) |
| 加编号、时间戳，或按来源序号重排 | [转换日志](guides/transform.md) |
| 显示日志或保存文件、JSONL、SQLite | [输出与保存](guides/archive.md) |
| 判断保存与停止结果 | [完整性与迁移](guides/recovery.md) |

具体命令和字段见 [CLI](reference/cli.md) 与 [配置](reference/configuration.md)。出现问题见 [排查](troubleshooting.md)。串口、复杂 TUI、WebUI 不在本版范围，独立 input-replay 与 io-plugin-util、log-plot 已取消。
