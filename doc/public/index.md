# log_print 使用手册

log_print 将程序输出和日志文件采集为可独立读取的日志流，再按需要转换或归档。你可以通过命令行查看数据，也可以让脚本和 AI Agent 读取带游标的结构化记录。

本手册对应 **0.1.1**。先从一条日志跑通流程，再按任务配置自己的数据源。

## 开始使用

- [安装与构建](installation.md)：从源码构建主程序和六个官方插件。
- [快速开始](quickstart.md)：跟随一个日志文件，读取新增内容并停止采集。
- [理解流与保存](concepts.md)：理解原始字节、记录、派生流，以及两种保存方式。

## 选择你的任务

| 你想做什么 | 阅读 |
| --- | --- |
| 收集自己运行的程序的 stdout 和 stderr | [采集程序输出](guides/program.md) |
| 持续读取应用写入的日志文件 | [跟随日志文件](guides/file.md) |
| 把已有日志送入同一套处理流程 | [回放已有日志](guides/replay.md) |
| 读取日志、接入脚本或 AI Agent | [读取与管理实例](guides/read.md) |
| 改编码、替换文本或转换字节表示 | [转换日志](guides/transform.md) |
| 导出原始字节，或保存可恢复的归档 | [输出与归档](guides/archive.md) |
| 继续已有归档、确认安全停止 | [恢复与完整性](guides/recovery.md) |

## 当前版本提供什么

输入包括程序、普通文件和文件回放；输出包括原始字节、转换后的派生流，以及文件、JSONL、SQLite 归档。串口、TUI 和 WebUI 暂缓，不在本版本默认构建范围内。

主程序和插件运行不需要 Python 或 Node.js；本手册用 Python 3 构造少量教学输入。AI Agent 可调用现有 CLI，但本版本没有额外的自然语言查询服务。

查具体字段时使用 [配置参考](reference/configuration.md)、[CLI 参考](reference/cli.md) 和侧栏中的插件参考。遇到问题先看 [排查常见问题](troubleshooting.md)。
