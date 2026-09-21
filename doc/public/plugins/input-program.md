# input-program：启动程序与 tmux 接入

[使用步骤](../guides/program.md) · [公共配置结构](../reference/configuration.md)

每个实例写入一条 Core 分配的流。两种模式的所有配置均为启动快照；更新主配置后需重启主程序，重启单个插件不重读配置。

## 启动模式

```json
{"mode":"spawn","command":"python3","args":["-u","app.py"],"chunk_bytes":4096,"shutdown_ms":2000}
```

| 字段 | 默认与范围 |
|---|---|
| `mode` | `spawn`；另一取值为 `tmux` |
| `command` / `args` | 启动模式必填可执行文件；参数数组默认空，不隐式调用 shell |
| `cwd` / `env` | 默认继承工作目录和环境；可设置目录和环境覆盖项 |
| `chunk_bytes` | 4096，1–65536；UDP 编码后还受包大小限制 |
| `shutdown_ms` | 2000，10–10000；终止后等待源进程回收的毫秒数 |

使用 stdout/stderr 管道，不分配 PTY，stdin 关闭。记录保留原始字节，不等待换行；`channel` 区分 `stdout` 与 `stderr`。每个通道有独立递增 `source_seq`，Core 的 `seq` 只代表实际接收顺序，不能还原跨管道的来源先后。源程序的缓冲由其自身 flush/无缓冲选项控制。

目标不继承插件连接使用的 `LOG_PRINT_*` 环境项。非零源退出或检测到 Core 连接错误时报告失败。UDP 没有 TCP 的 EOF 通知，Core 进程崩溃由主程序检测并组织清理。手动停止会终止本插件创建的 Unix 进程组或 Windows Job，并尝试回收后代；主动脱离 Unix 进程组的守护进程不在回收范围内。若后代继续持有输出管道，采集会等待管道关闭或手动停止。

## tmux 模式

```json
{"mode":"tmux","tmux_target":"%3","chunk_bytes":4096}
```

要求 Unix 与可用的 tmux；`tmux_target` 必填，建议使用明确的窗格 ID。可选 `tmux_socket` 为服务器 socket 路径，对应 `tmux -S`；不指定则使用当前 tmux 环境或默认服务。此模式拒绝 `command/args/cwd/env`，目标程序不会为了接入而重启。

只采集接入后的新输出，不导入历史屏幕。记录为终端字节，可能含控制码；`channel` 为 `terminal`，无法还原 stdout/stderr 区分。已有 `pipe-pane` 时拒绝，通过 tmux 同步条件命令处理检查与安装，避免竞争期间关闭他人的管道。

专属辅助进程通过私有 Unix socket 传输；停止只断开自己的连接，辅助进程退出，目标继续运行。后来由别人替换管道时，清理也不会关闭新管道。本模式不提供任意 PID、普通 TTY 附着或全历史恢复。

两种模式的 EOF 都不代表下游完成。有限滚动缓冲可能覆盖记录，UDP 本地发送不代表 Core 接收，均不承诺完整无损。旧 `stdout_stream/stderr_stream` 双流配置已移除。
