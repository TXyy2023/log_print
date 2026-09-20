# input-replay：回放输入

先完成任务教程：[使用步骤](../guides/replay.md)。本页用于查询参数和行为边界。

将既有文件透明回放为 Core 普通流，原负载逐字节不变。

下列 JSON 是插件声明中的 `config`，不是完整启动配置。发布流须列入 `streams`，读取流须列入 `reads`；公共结构见 [配置参考](../reference/configuration.md)。

插件接受 `shutdown`、`config.get`。标为动态的字段可通过 CLI `config set` 修改，只影响当前进程；其他字段需更新配置并重启实例加载。

## 配置

```json
{"path":"project/examples/io/replay.log","stream":"replay","timestamp":"auto","interval_ms":10,"speed":1.0,"chunk_bytes":4096,"max_gap_ms":60000}
```

| 字段 | 默认/范围 | 生效 |
|---|---|---|
| path / stream | path 必填；stream=replay | 重启 |
| timestamp | auto / none / rfc3339 / unix_ms / unix_ns | 重启 |
| interval_ms | 10，0..60000；无解析时间的模拟行节奏 | 动态 |
| speed | 1.0，0.01..1000 | 动态，下个间隔起 |
| chunk_bytes | 4096，1..65536 | 重启 |
| max_gap_ms | 60000，1..3600000；缩放后最大等待 | 重启 |

## 行为与限制

`auto` 解析行首 RFC3339 时间，或完整 JSON 行的 `source_ts_ns` / RFC3339 `timestamp` 字段。显式 unix 模式解析首个空白分隔的无符号整数。时间解析复用 [chrono](https://docs.rs/chrono/latest/chrono/)。缺失/不可解析时间使用模拟间隔；倒退或等待截顶报告状态。第一行立即发，后续按相邻可解析行的间隔除以 speed。超长行拆成有界块，续块立即发，不改负载、不重复等待；极小 chunk 无法容纳时间前缀时明确作为模拟节奏，不能把部分前缀猜成源时间。

每次运行新 UUID，Core/Output 没有 replay 专用业务分支。回放结束报告字节数、源时间行数与模拟行数，再正常退出；不循环自动重启。
