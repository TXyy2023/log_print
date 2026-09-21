---
name: log-print
description: 用 log-print CLI 启动本地日志采集、读取流和停止实例。适用于本项目文件、程序与 tmux 日志采集，不包含自动监控或复杂 Agent 编排。
---

# log-print

适用于 0.1.2 / log-print/2。先在项目根目录按 README 构建 `target/release/log-print`（Windows加.exe）。配置参考在 `doc/public/reference/configuration.md`。

流程：使用 `start --config FILE` 启动，`streams` 取得 Core 分配的真实 UUID，`read UUID --raw --wait-ms 1000` 读取当前保留快照；持续消费使用预配置 Output，执行 `plugin start ID --stream UUID`。最后对自己启动的实例执行 `stop`。

`--state PATH` 选择明确实例，不启动重复实例代替选择，不删除正在运行的状态文件。`status` 查看插件报告和实际进程退出结果。配置是启动快照，修改后必须重启主程序；无config set或session界面。

Core只有有界滚动内存，不保存历史。UDP成功只表示本地发送，TCP接受也不代表Output处理或保存。`read` 不是增量游标，重复查询可能重复；缓冲覆盖后数据无法由Core恢复。错误、forced停止和未知结果需如实报告；不要盲目重发。原流不变，转换发布到独立派生流。
