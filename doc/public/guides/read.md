# 读取与管理实例

```sh
log-print streams
log-print stream STREAM_UUID
log-print read STREAM_UUID --limit 64
log-print read STREAM_UUID --raw --wait-ms 1000
```

先读取 `streams` 返回的 `id`，不要把配置别名当运行时 UUID。`read` 只取当前仍保留内存的快照；重复调用可能重复，不能作为无损增量查询。空流配合 `--wait-ms` 等待到截止时间；非空立即返回。

持续消费请启动 Output；可事先配置 `autostart:false`，在流创建后执行：

```sh
log-print plugin start screen --stream STREAM_UUID
log-print plugin stop screen
```

独立 Output 从最早保留记录开始，读到末尾等新数据。停止一个 Output 不关闭整条流，也不停止 Input。Input 自身退出后缓冲仍保留。`status` 同时提供插件业务报告与实际子进程状态；停止请求被接受不等于进程已完成。

AI Agent 应保留结构化结果中的流 ID、来源序号和通道信息；不要从 UDP 本地发送或 Core 内存接受推断日志已持久保存。详见 [CLI](../reference/cli.md)。
