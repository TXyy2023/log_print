# output-transform：加工与派生流

订阅原流，以 Output 身份向另一条单写入者派生流发布加工结果。原流 payload 与记录保持不变。使用教程见[转换日志](../guides/transform.md)。

```json
{"streams":["source"],"output_stream":"derived","number":true,"timestamp":true,"reorder":true,"max_records":128,"max_bytes":4194304,"max_delay_ms":100,"max_channels":128}
```

主配置须声明 `role:"output"`、授权 `reads` 和独立派生 `streams`。实际绑定 reads 优先于 config.streams。派生流不能是原流，也不能借知道 UUID 取得其它流的写入资格。

| 字段 | 默认及行为 |
| --- | --- |
| streams / output_stream | 可省略；使用注册时授权的读取流和派生流 UUID |
| number | false；在记录前加 `[n=1] `，本次插件运行从 1 递增 |
| timestamp | false；加 `[ts_ns=...] `，来源纳秒时间优先，否则 Core 观察时间 |
| reorder | false；true 按来源序号有限等待重排 |
| max_records | 128；缓冲总条数 1–4096 |
| max_bytes | 4 MiB；缓冲序列化总大小，64 KiB–64 MiB |
| max_delay_ms | 100；缺号最多等待 1–60000 ms 后触发发布 |
| max_channels | 128；最多 1–4096 个来源状态，超出明确失败 |

默认不启用加工时 payload 原样派生。开启编号/时间戳后仍是每条输入一条输出；结果超过协议 payload 上限明确失败，需减小输入块。不会隐式拆成多条或执行任意用户代码。

## 重排、重复与缺号

重排键为 `(stream,epoch,channel)`，每组期望 `source_seq` 从 1 开始。程序 stdout/stderr 各有独立来源序号，不假定两者原本存在共同顺序。没有 `source_seq` 的记录按到达顺序直接发布并计数。

重复来源号保留第一条；已发布位置之前的迟到号丢弃。缺号时有限等待；超过窗口或缓冲预算将满时，发布相应组当前最小号和之后连续的记录，统计跳过的来源号。正常停止先结束接受，再排空已接受事件和待重排记录。此策略不能恢复未到达或被 Core 覆盖的数据。

每条派生记录的 `upstream` 只指向它实际来自的原流/Core 序号，不捏造其他流进度。派生记录保留 channel，并在每个输出 channel 内生成从 1 递增的 source_seq。编号前缀则是整个转换实例的发布编号。

首次从当前缓冲最早保留记录订阅，读到末尾后持续等待。Input EOF 不是自动停止指令。配置只在主进程重启后更新；本版没有旧版编码、正则替换或动态配置能力。
