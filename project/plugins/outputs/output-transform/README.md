# output-transform 0.1.2

订阅原流，以 Output 身份发布另一个单写入者派生流。Core 流记录保持不变。

配置示例：`{"streams":["source"],"output_stream":"derived","number":true,"timestamp":true,"reorder":true,"max_records":128,"max_bytes":4194304,"max_delay_ms":100,"max_channels":128}`。

`number` 在每条派生记录前加 `[n=1] `，从本次插件运行的 1 开始；`timestamp` 加 `[ts_ns=...] `，先选来源纳秒时间，否则用 Core 观察时间。开启加工后大于协议 payload 上限会明确失败，请减小输入块。

重排只在 `reorder:true` 时发生，以 `(stream, epoch, channel)` 分组使用 `source_seq`，每组期待从 1 开始。不设总来源时间顺序。重复号保留第一条；已发布之前的迟到号丢弃。遇到缺号，按最早待重排记录的接收时间加 `max_delay_ms` 设置到期唤醒（实际发布另受调度和传输影响）；记录数或字节预算将满时提前发布该组当前最小序号及随后连续记录，停止时按同一规则排空，并统计跳过的来源序号。`max_channels` 分别限制来源状态和派生通道计数状态数量，关闭重排或缺少来源序号时同样生效；超限明确失败。没有 `source_seq` 的记录按到达顺序直接发布，并计数。

默认订阅 Core 当前保留的最早记录，之后等待新数据。重排不恢复已丢失或被 Core 覆盖的数据。配置是主进程启动快照，修改需重启主进程。
