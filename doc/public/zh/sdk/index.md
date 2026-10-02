# SDK 总览 {#sdk-overview}

SDK 通过 `log-print/2` 将插件代码接入 Core。Core 负责身份、权限、流 UUID 和有界内存；采集、转换、显示与持久保存由插件负责。

## 选择 SDK {#choose}

| SDK | 交付状态 | 入口 | 传输 |
| --- | --- | --- | --- |
| [Rust SDK](rust.md) | 已包含在 main / 0.1.3，crate 为 `log-plugin-sdk` | `connect_env()` 或 `connect(validate)` | TCP 或 UDP |
| [Python SDK](python.md) | 已在 `codex/python-sdk` 分支实现，提交 `6be6992`；未包含在 main / ver-0.1.3 | `async with log.init() as app` | 仅 TCP |

Python 分支状态核对于 2026-10-03。请按其指南从指定源码版本安装；main 工作区没有 `project/sdks/python` 目录。本文不假定已有公开包索引发行版。

## 通用接入流程 {#workflow}

1. 在[实例配置](../reference/configuration.md)声明插件可执行文件、角色和流权限。
2. supervisor 启动进程，通过 `LOG_PRINT_*` 环境变量交接 Core 地址、身份、凭据和配置。
3. Input 向自有流发布字节；Output 订阅获准读取的流；转换插件是带自有派生流及父流声明的 Output。
4. 协作处理停止，完成业务收尾，再报告真实业务结果。

Rust SDK 连接已有 Core。Python 还提供独立本地模式：启动已安装的 Core 二进制，退出时仅清理自身实例。不同独立会话不共享流。

## 交付与生命周期边界 {#guarantees}

- TCP 接受仅表示 Core 接收了记录，不表示消费者已处理或保存。Rust UDP 的 `LocalSent` 仅表示本地发送完成。
- 订阅先读取保留记录，再等待新记录；Core 内存可能覆盖未读历史，没有缺口事件不能证明完整。
- 范围读取查询 Core 保留内存。持久历史属于 [output-file](../plugins/output-file.md)，不是 SDK 的能力。
- 不提供自动重连、重试、持久 outbox 或 exactly-once。超时或取消可能使远端结果未知。
- 停止确认不等于业务完成；修改配置需要重启。

继续阅读 [Rust API 与示例](rust.md)或 [Python 安装、示例与 API](python.md)。
