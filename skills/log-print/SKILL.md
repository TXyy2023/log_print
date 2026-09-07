---
name: log-print
description: 用 log-print CLI 启动本地日志采集、读取流和停止实例。适用于本项目日志与串口只读采集，不包含自动监控或复杂 Agent 编排。
---

# log-print

先在项目根目录运行命令；使用已构建的 `target/release/log-print`（Windows 加 `.exe`）。配置和命令入口见项目 `README.md`、`doc/cli.md`。

最小完整流程：

```sh
python3 -c "from pathlib import Path; Path('example.log').touch()"
./target/release/log-print start --config examples/basic.json
python3 -c "open('example.log','ab').write(b'temperature=23.5\n')"
./target/release/log-print read logs --raw --wait-ms 1000
./target/release/log-print stop
```

常用命令：`status`、`streams`、`read STREAM`、`plugin start|stop|restart ID`、`session list PLUGIN`、`session get PLUGIN ID`。用 `--state PATH` 明确选择已有实例；不要启动重复实例来代替选择。

默认不保存，读取只覆盖当前缓冲；检查 JSON 的 epoch、next、gap 和保存状态。读到 gap、Disconnected 或 storage_blocked 时明确报告；不把未知结果当成功，不更换 key 盲目重发。源字节保留，转换只写派生流。结束自己启动的采集时执行 stop。
