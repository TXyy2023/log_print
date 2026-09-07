# input-replay

将既有文件透明回放为 Core 普通流，原负载逐字节不变。

所有插件由 `log-print` 注入 `LOG_PRINT_CORE/PLUGIN/TOKEN/CONFIG`，通过 Rust SDK 的本地 TCP IPC 连接 Core；stdout 不承载协议，诊断写 stderr。下列 JSON 放在插件声明的 `config`。流须同时出现在声明的 `streams`/`reads`，保存由 Core 按声明执行。默认读到即推，不等待换行；每次发布保留原 key 等待明确保存恢复，其他错误退出且报告。

所有插件接受 `shutdown`、`config.get`；下表列出的动态字段接受 `config.patch`，只改本次进程、不写配置文件。其余字段重启插件生效。

```json
{"path":"examples/io/replay.log","stream":"replay","timestamp":"auto","interval_ms":10,"speed":1.0,"chunk_bytes":4096,"max_gap_ms":60000}
```

| 字段 | 默认/范围 | 生效 |
|---|---|---|
| path / stream | path 必填；stream=replay | 重启 |
| timestamp | auto / none / rfc3339 / unix_ms / unix_ns | 重启 |
| interval_ms | 10，0..60000；无解析时间的模拟行节奏 | 动态 |
| speed | 1.0，0.01..1000 | 动态，下个间隔起 |
| chunk_bytes | 4096，1..65536 | 重启 |
| max_gap_ms | 60000，1..3600000；缩放后最大等待 | 重启 |

`auto` 解析行首 RFC3339 时间，或完整 JSON 行的 `source_ts_ns` / RFC3339 `timestamp` 字段。显式 unix 模式解析首个空白分隔的无符号整数。时间解析复用 [chrono](https://docs.rs/chrono/latest/chrono/)。缺失/不可解析时间使用模拟间隔；倒退或等待截顶报告状态。第一行立即发，后续按相邻可解析行的间隔除以 speed。超长行拆成有界块，续块立即发，不改负载、不重复等待；极小 chunk 无法容纳时间前缀时明确作为模拟节奏，不能把部分前缀猜成源时间。

每次运行新 UUID，Core/Output 没有 replay 专用业务分支。回放结束报告字节数、源时间行数与模拟行数，再正常退出；不循环自动重启。
