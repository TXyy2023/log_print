# output-raw：终端日志展示

订阅 Core 流并写到插件 stdout。保存日志使用 [output-file](output-file.md)，加工内容使用 [output-transform](output-transform.md)。

```json
{"streams":["source"],"annotate":false}
```

| 字段 | 行为 |
| --- | --- |
| streams | 可选流 alias / UUID 数组；主配置 `reads` 或 CLI 实际绑定优先 |
| annotate | 默认 false；true 在每条记录前显示流 UUID、Core 接收序号、来源通道 |

默认逐条写出 payload 原始字节，不添加换行，支持二进制和空 payload。多流按本插件收到的顺序交错；不声明全局时间顺序。主进程可以重定向插件 stdout，实际位置以启动状态显示为准。

订阅从 Core 当前缓冲最早保留的记录开始，读到末尾后持续等待，Input 暂离或空闲不会触发正常结束。停止由用户手动触发。滚动覆盖和 UDP 丢包可能导致数据缺失，终端输出不承诺完整性。

本版本不接受 `path`、`paths`、`from`、`append` 或 `overwrite`。配置是主进程启动快照，`config.get` 可查看；修改文件需重启主进程，`config.patch` 返回 restart_required。
