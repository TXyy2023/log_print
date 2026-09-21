# CLI 参考

```text
log-print [--state STATE] COMMAND
```

默认状态路径 `.log-print/state.json`。成功输出 JSON，失败向 stderr 报错并返回非零；`read --raw` 输出 payload 字节。

| 命令 | 作用 |
| --- | --- |
| `run --config FILE` | 前台启动，Ctrl-C 停止自有进程 |
| `start --config FILE` | 后台启动，等就绪后返回 PID、日志路径和流 ID |
| `status` | Core、插件报告、子进程退出状态 |
| `streams` | 列出流 UUID、说明、归属与缓冲状态 |
| `stream UUID` | 查看一个流对象 |
| `read UUID [--limit 64] [--wait-ms 0] [--raw]` | 当前保留缓冲的快照，limit 1..64；空结果可等待 0..60000 ms |
| `config [--plugin ID]` | 读取运行时采用的启动快照 |
| `plugin start ID [--stream UUID]` | 启动预配置插件；可把未连接 Output 关联到真实流 ID |
| `plugin stop ID` | 请求收尾并等待子进程退出，返回 success/forced |
| `plugin restart ID` | 停止后使用同一启动快照重新启动 |
| `plugin call ID METHOD [--json JSON]` | 插件控制方法，如 `config.get`、output-file 的 `status.get` |
| `call OP [--json JSON]` | 高级 Core 对象/RPC 操作 |
| `stop` | 停止实例并等待状态文件移除 |

示例：`call stream.describe --json '{"stream":"UUID","description":"编译日志"}'` 修改显示说明，不改变身份或内容。

`read` 每次从当前最早保留记录取快照，不提供持久历史游标，重复执行可能读到相同记录。需要持续消费使用 Output/SDK 订阅。旧 `--from`、`--epoch`、`config set`、`session` 已移除。

状态文件包含管理凭据，不要提交。已有实例占用状态文件时，新启动会失败；不要删除正在运行实例的状态文件绕过检查。配置文件修改只有主程序重启后生效，不提供 reload 或热配置。

停止命令返回实际收尾结果；任一插件失败或强制终止时CLI返回非零，restart在停止失败后不再拉起新进程。任意卡死Unix插件若最终被SIGKILL，其内部清理无法执行；此时来源进程清理状态未确认，不表示完整保存或完整树回收。
