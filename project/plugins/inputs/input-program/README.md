# input-program

0.1.2 支持由 CLI 启动程序、接入已有 tmux 窗格两种方式。每个实例只写入一条 Core 分配的流；别名与说明放在 `PluginSpec.streams`。主程序通过 `log-print start --config ...` 组织插件运行。

```json
{"id":"program","role":"input","bin":"input-program","streams":[{"id":"program-data","description":"目标程序输出"}],"config":{"mode":"spawn","command":"python3","args":["-u","app.py"]}}
```

`spawn` 为默认模式，使用独立 stdout/stderr 管道，不分配 PTY，stdin 为关闭状态。原始字节保留，记录 `channel` 为 `stdout` 或 `stderr`；每个通道有独立递增 `source_seq`，合流后的 `seq` 只代表 Core 接收顺序，不代表两个管道的来源产生顺序。子程序自身的缓冲会影响采集时机。

可选 `cwd`、`env`、`chunk_bytes`（1–65536，默认 4096）、`shutdown_ms`（10–10000，默认 2000）。继承的 `LOG_PRINT_*` 环境项在启动目标时移除。非零退出报告失败；停止插件会终止本插件创建的进程组（Windows 使用 Job Object）并尝试回收，因此也会停止仍运行的后代。若后代仍持有输出管道，插件会继续采集，直到管道关闭或手动停止。

```json
{"id":"pane","role":"input","bin":"input-program","streams":[{"id":"pane-data","description":"已有 tmux 程序"}],"config":{"mode":"tmux","tmux_target":"%3"}}
```

`tmux` 模式要求 Unix 与可用的 tmux。`tmux_target` 指定窗格 ID 或可解析目标，建议使用明确的 `%窗格ID`；`tmux_socket` 可指定独立服务器 socket 路径（相当于 `tmux -S`）。本模式不接受 `command/args/cwd/env`，不会重启或改变目标程序。

只从管道接入时采集新输出，不导入历史屏幕；字节属于终端输出，可能包含控制码，`channel` 为 `terminal`，无法还原 stdout/stderr 区分。已有 `pipe-pane` 时拒绝接入；通过 tmux 同步条件命令检查并安装管道，并发出现已有管道时也拒绝接入。专属辅助进程通过私有 Unix socket 传输，停止只关闭自己的连接，由辅助进程退出解除采集，目标程序保持运行。后来替换为其他管道时，也不会在清理中关闭那个新管道。

所有配置在启动时固定。源 EOF/发布调用完成不代表 Output 处理完成；Core 有限滚动缓冲与 UDP 均不保证完整交付。不支持任意 PID 附着、普通已运行 TTY 或历史恢复；旧 `stdout_stream/stderr_stream` 双流字段已移除。
