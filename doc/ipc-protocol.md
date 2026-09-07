# log-print/1 IPC

首个正式协议和 Rust SDK，不保证后续版本兼容；不兼容变更显式升级 `protocol`，不悄悄接受旧客户端。与 MVP 的旧 `version:1/2` 消息不兼容。

所有连接仅监听 loopback。每个 JSONL 帧至多 1 MiB，原始 payload 至多 64 KiB；即使没有帧换行，也会在超限时拒绝。类型定义以 `crates/log-proto/src/lib.rs` 为准。

## 握手与消息

```json
{"protocol":"log-print/1","plugin":"reader","token":"实例分配的插件令牌","events":false}
{"type":"response","id":0,"result":{"protocol":"log-print/1","plugin":"reader"},"error":null}
{"id":1,"op":"read","args":{"stream":"logs","from":1,"limit":64}}
```

`events:false` 为请求/控制连接；每个插件只有一个活动请求连接。`events:true` 为单流订阅连接，握手后发送一次 subscribe。管理员标识 `__admin__` 使用独立令牌，可同时执行 CLI 操作。令牌由主进程提供，不得写入日志或提交配置样例。

响应：`{"type":"response","id":1,"result":{...},"error":null}`。失败为 `result:null,error:{code,message}`。请求 id 用于关联，不代表保存事务身份。

| op | args | 结果 |
|---|---|---|
| status | `{}` | pid、protocol、流范围/保存/阻塞状态、插件连接/报告和配置 |
| publish | stream、key、payload 字节数组、source_ts_ns（可空）、upstream 父流→seq | 完整 Record；只允许 owner |
| read | stream、from（默认1）、limit（1..64）、epoch（可选） | records、next、head、oldest、epoch、gap |
| subscribe | 与 read 相同，在 events 连接使用 | 首先确认，再推送 record/gap；无定时读取 |
| resume | stream | 管理员手动重开受影响存储；结果为流状态 |
| config.patch | buffer_bytes / buffer_records / read_batch_records | 管理员运行时覆盖，仅安全项 |
| report | 有界 JSON 对象 | 插件运行状态，至多16 KiB |
| control | target、method、args | 管理员向插件路由调用，10秒未确认返回未知结果 |
| reply | call_id、result、error | 目标插件回复此前控制调用 |

Control 事件为 `{"type":"control","call_id":7,"method":"shutdown","args":{}}`。插件控制方法由其 README 定义，共同方法包括 shutdown/config.get；TUI/WebUI 实现 sessions 与 session.create/get/patch/export。

## 记录与读取

```json
{"stream":"logs","epoch":"实例或持久流UUID","seq":1,"key":"producer-run:chunk-1","payload":[65,0,255],"source_ts_ns":null,"observed_ts_ns":1,"upstream":{},"upstream_epochs":{},"durability":"buffered"}
```

`source_ts_ns` 是调用者提供的 Unix 纳秒；未知时留空。`observed_ts_ns` 由 Core 提供，不等于设备原始时间。保存成功记录使用 `durability:"saved"`，未保存使用 `"buffered"`。派生请求的 upstream 恰好覆盖声明的所有 parents；Core 拒绝父 seq=0 或超过已知 head，并补父 epoch。

from=1 从最早逻辑记录开始；from=0 在本次建立读取/订阅时只取 head+1 后的新记录。保存游标建议保存 epoch 和 next。历史每页同时限制条数和 512 KiB 序列化记录预算，可能少于 limit；使用 next 继续，不能把“少于 limit”误判为已读完。

未保存流覆盖旧缓冲时，read.gap 或 gap 事件给出被覆盖的闭区间 `[from,to]`，next 跳至可用位置。传输断开由 SDK 单独发 Disconnected，缺失量未知，不伪造成确定的 gap 条数。历史损坏/缺失返回错误，不以空 records 冒充正常追平。

保存流同 key 同 payload/source_ts/upstream/父 epoch 返回原记录，同 key 不同内容返回 key_conflict。保存流的幂等记录随可用历史保留；未保存流仅在仍保留的缓冲范围内去重，不提供跨重启/覆盖后的持久幂等保证。SDK 官方 Input 一次保留当前未确认块；无确认时不能换 key 盲重发。

## 失败与边界

| code | 意义与处理 |
|---|---|
| version_mismatch / authentication_failed / already_connected | 握手被拒绝；修正版本、实例身份或关闭旧连接 |
| permission_denied / invalid_parents / invalid_parent_cursor | 修正声明或来源，不能静默忽略 |
| limit / cursor_ahead / epoch_mismatch | 修正负载、分页或游标，epoch 变化需重新决定起点 |
| storage_blocked | 立即报告；保留当前数据并等管理员修复/resume，不自动关闭保存 |
| commit_unknown / timeout_unknown / connection_lost | 结果不明；原 key 与原内容保留，不能宣称已丢失或已保存 |
| history_unavailable / history_gap | 当前不能满足完整读取；检查损坏/缺段/身份，不把错误变成正常空结果 |
| control_busy / control_timeout / not_connected | 控制未得到明确成功；检查插件状态，非幂等操作勿自动重放 |

主程序管理协议复用握手/请求/响应外壳，包含 plugin.start/stop/restart 和 core.call；日常使用 CLI。SDK 封装数据 IPC，不控制其他插件的操作系统进程。
