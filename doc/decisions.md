# 实施者决定

这些是为已授权需求作出的实现决定，不声称用户逐一指定了库或参数。

| 决定 | 采用理由、替代与边界 |
|---|---|
| 官方插件统一 Rust workspace | 正式仓库可一次构建、锁版本、验收和贡献；旧独立仓库完整归档，第三方仍可独立实现协议 |
| loopback TCP、有界 JSONL、独立订阅连接 | 三平台无需自写命名管道兼容层，成熟 Tokio 支持背压；stdout留给原始输出/TUI。字节数组有编码开销，性能实测决定是否后续升级二进制协议 |
| SDK 独立完整帧发送队列 | 取消等候不能截断线上帧；慢事件与发布/控制隔离。不是无限队列或自动恢复机制 |
| Core 内部每流 SQLite 分段 | 复用事务、索引、校验和、故障语义；不用旧外部 Python storage。DELETE+EXTRA 便于计算事务临时空间；不声称拥有数据库自身无法提供的硬件保证 |
| 显式保存、失败阻塞、手动恢复 | 保持数据状态可解释，不在故障后静默变为只缓冲；未保存环形缓冲有明确覆盖反馈 |
| 持久分段目录与提交 head | 独立审查实证尾段/全段删除可能导致错误缩短历史，因此加入检测并回归；完整目录被人为回滚仍需外部备份策略 |
| 程序通用管道、文件跟随、serialport | 一套 Input 可覆盖多语言 stdout/stderr；文件截断/替换分段报告；串口只读取，覆盖不等于所有硬件协议都实测 |
| encoding_rs/chardetng/regex | 成熟流式编码和非回溯正则，避免手写字符表；自动猜测保守保留不确定字节，允许显式人工覆盖 |
| Ratatui+Crossterm、Axum+ECharts | 分别适配终端和浏览器；两个独立插件、同等首版能力，采用成熟曲线组件，无网络CDN依赖 |
| Plotters+内嵌授权字体导出 | PNG/SVG不依赖外部浏览器或系统字体；中文资产增加安装和SVG体积，实际大小与资源报告保留 |
| session revision 和有界图表窗口 | 防止并发CLI覆盖，导出冻结对应配置和可用范围；显示窗口/抽样不改变原始Core数据 |
| Windows后台CLI清除调用者stdio继承位 | Rust普通spawn会继承额外可继承句柄，重定向日志不足以保证调用者PIPE得到EOF；仅清本CLI有效标准句柄的INHERIT标志，不关闭或替换句柄，指定日志/NUL由Rust正常复制。用真实PIPE start/status/read/stop而非仅文件输出验收 |
| 无 watchdog、自动重启和额外云服务 | 保持首版范围；正常退出、错误反馈、按需启停和验收测量不等于运行时监控 |

选型核对来源：[Tokio framing](https://tokio.rs/tokio/tutorial/framing)、[Tokio channels](https://tokio.rs/tokio/tutorial/channels)、[SQLite pragma](https://www.sqlite.org/pragma.html)、[serialport](https://docs.rs/serialport/latest/serialport/)、[Ratatui](https://docs.rs/ratatui/0.30.2/ratatui/)、[Axum SSE](https://docs.rs/axum/0.8.9/axum/response/sse/)、[ECharts 6.1.0](https://github.com/apache/echarts/releases/tag/6.1.0)、[Plotters](https://docs.rs/plotters/0.3.7/plotters/)。实际 Rust 版本锁定在 Cargo.lock；源码及许可证见 THIRD_PARTY.md。

Windows机制与API依据：[Rust 1.98 process实现](https://github.com/rust-lang/rust/blob/1.98.0/library/std/src/sys/process/windows.rs)、[SetHandleInformation](https://learn.microsoft.com/en-us/windows/win32/api/handleapi/nf-handleapi-sethandleinformation)。实现使用稳定windows-sys绑定，未启用Rust不稳定进程属性接口。
