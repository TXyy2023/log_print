# 正式版实现契约（2026-09-08）

本文件记录实施者采用的 P1 设计。实施已授权，原始清单保持不动。

- 官方插件统一进入 Cargo workspace，历史独立 Git 仓库完整保留于本地归档。Core/SDK 不依赖具体插件。
- 主程序 `log-print` 管理同目录 `log-print-core` 与插件可执行文件。Core 与所有插件均为主进程直接子进程。CLI 通过本地管理地址控制当前实例。
- Core 绑定随机 loopback TCP，开启 TCP_NODELAY，采用有界 JSONL，请求 `id/op/args`；握手 `protocol=log-print/1`。每个插件有独立令牌与配置声明的读写权限，管理员令牌位于权限受限运行状态文件。版本不兼容明确拒绝。
- 每个 SDK 一条请求/控制连接，加每流一条事件连接。数据慢消费不能堵塞发布确认和 CLI 控制；网络、应用队列、单条及历史批次均有界。TCP 只用于本地 IPC，不开放远程服务。
- `log-proto` 是共享契约；保存默认不开启，配置优先 Core 默认 → 插件保存选项 → 流保存选项。单流 head 从 1 开始，next 表示下一条。未保存 Core 重启产生新 epoch，保存流保留 epoch。
- SDK `connect_env() -> (Client, Receiver<Event>, Receiver<Control>)`。Client 可 Clone，`config()` 返回插件 JSON 配置；`request(op,args)`、`subscribe(stream,from)`、`publish(stream,key,payload,source_ts_ns,upstream)`、`reply_control(call_id,result,error)`。事件 `Record(Record)` / `Gap { stream, epoch, from, to, reason }` / `Disconnected { stream, reason }`；Control 有 call_id/method/args。
- 环境变量：`LOG_PRINT_CORE`、`LOG_PRINT_PLUGIN`、`LOG_PRINT_TOKEN`、`LOG_PRINT_CONFIG`。诊断 stderr，插件 stdout 专供原始 Output/终端显示，不承载协议。
- Core RPC：status、read(stream,from,limit,epoch?)、subscribe(stream,from,epoch?)、publish(stream,key,payload,source_ts_ns?,upstream?)、resume(stream)、report(status任意对象)、control(target,method,args)、reply(call_id,result,error)。管理员可读取所有流，插件只能 reads 声明。发布只能写自己的流。派生流要求 parents 声明且父游标有效，Core记录对应upstream_epochs。显式父关系和插件实际reads依赖共同防环。
- Core 的 `control` 将调用交给目标插件并等待 reply，最多 10 秒；不自动重启。`shutdown` 为插件统一控制方法。状态查询可观察插件报告和连接状态。
- 图形插件各自处理 `sessions`、`session.get`、`session.create`、`session.select`、`session.patch`、`session.export`，参数由 UI 实现细化，Patch 带 revision 以免并发覆盖。只变对应 session，导出冻结数据与配置。
- Core 可执行入口：`log-print-core --runtime-config <RuntimeConfig JSON 文件> --ready-file <路径>`，启动成功写 `{address,pid}`，后台令牌不写入日志。管理父进程关闭 stdin 后 Core 退出。

传输参考 [Tokio framing](https://tokio.rs/tokio/tutorial/framing) 与 [bounded channels](https://tokio.rs/tokio/tutorial/channels)。保存使用 SQLite 事务；采用分段SQLite DELETE/EXTRA、独立目录锁、段目录清单和事务head；同步设置和容量定义见[配置规范](configuration.md)，不把 API 返回或测试模拟当真实断电证明。

SDK采用独立有界写入器，取消调用不会截断JSON帧；正常与控制回复队列各32帧，请求总deadline为30秒。最终可测协议以[IPC规范](ipc-protocol.md)和[SDK说明](sdk.md)为准。
