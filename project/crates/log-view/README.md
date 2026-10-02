# log-view

WebUI 与 TUI 的共享显示库，编译进两个 Output，**不是额外服务**。Core 仍只负责流和内存读取，不依赖本库或 SQLite。

- `server.rs`：当前 SDK 会话、全流订阅、loopback HTTP/SSE、同源校验、控制帧预算、停止清理。
- `store.rs`：独立 SQLite Page 配置、revision、原子 layout 提交、旧 GridStack 配置兼容。
- `engine.rs`：有界实时缓存、后台扫描、固定查询边界、output-file HistoryReader、合并去重及共享暂停帧。
- `lines.rs`：跨 Record/通道分行、身份与字节偏移、数值提取及缺口。
- `cli.rs`：WebUI CLI、TUI CLI 和终端命令面板共享的命名参数定义。

WebUI 只增加嵌入的 Vue 静态资源路由。TUI 后台只增加终端连接提示；其 attach 客户端不读取 Core 凭据，不创建流或发布记录。多个视图连接同一个后端，避免多进程并发打开 Page 数据库。

共享契约分别由 `quality/tests/v2/webui.py` 和 `tui.py` 对两个真实 Output 进程验证；前端和 PTY 验收覆盖各自适配器。代码调整必须让两种显示器都通过，而不能只测试本库。
