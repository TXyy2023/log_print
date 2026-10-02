# 0.1.3 发布与验收

0.1.3 汇总 ver-0.1.2 之后的 CLI、WebUI、TUI 和双语手册更新；协议仍为 log-print/2。旧标签不移动。

## 变化

- CLI 支持多流组织与显示工作台控制。
- output-webui 提供离线资源、持久页面、自由布局、表格和曲线，并通过 CLI 管理。
- output-tui 与 WebUI 共用显示引擎，提供真实终端键盘/鼠标交互和上下文查看。
- 显示层结合 Core 实时数据与 output-file SQLite 归档上下文。
- 英文为默认文档语言，提供完整中文手册与语言切换。
- 补齐跨平台 PTY/ConPTY、双浏览器同步与归档压力验收。

## 发布门槛

最终版本提交必须通过 [Formal validation](https://github.com/TXyy2023/log_print/actions/workflows/validate.yml) 的 Linux、macOS、Windows 任务，以及 [Documentation](https://github.com/TXyy2023/log_print/actions/workflows/docs.yml)。正式验收涵盖前端锁文件构建、Rust 格式/Clippy/测试、真实进程、PTY/ConPTY 和两个 Chromium 浏览器视图。

最终提交、CI 链接与校验结果记录在 [GitHub Release](https://github.com/TXyy2023/log_print/releases/tag/ver-0.1.3) 及其 verification.json 中；本页不将旧提交的通过结果当作本版证据。

## 使用与边界

从 ver-0.1.3 源码执行 cargo build --release --workspace --locked，程序与手册使用同一版本。GitHub 自动生成源码包；本次不提供预编译安装包。

Core 历史仍为有界内存，重启后丢失；持久历史由 output-file 负责。UDP 不提供逐条确认和重传；tmux 接入仅 Unix。软件验收不代表真实串口设备、断电恢复或任意负载下的性能保证。Python SDK 规划不属于本次发布的已实现功能。
