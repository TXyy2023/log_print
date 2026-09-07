# log_print

一个面向 CLI、AI Agent 和本地调试的日志工具。一个主进程管理一个独立 Rust Core 和多个独立插件：同时采集程序、文件和串口，读取原始字节，处理派生流，并用独立 TUI/WebUI 绘制数值曲线。

这是首个正式协议 `log-print/1`。实际验证与限制见 [验证报告](doc/validation.md)，不把依赖库支持的平台或历史 MVP 测试当成本版本已验证能力。

## 构建与最小使用

需要 Rust 1.92 或更新的兼容稳定工具链。验收脚本使用 Python 3.12+。官方插件随同一 Cargo workspace 构建，运行时无需 Python；示例源程序和测试使用 Python。

```sh
cargo build --release --locked --workspace
python3 -c "from pathlib import Path; Path('example.log').touch()"
./target/release/log-print start --config examples/basic.json
python3 -c "open('example.log','ab').write(b'temperature=23.5\n')"
./target/release/log-print status
./target/release/log-print read logs --raw --wait-ms 1000
./target/release/log-print stop
```

Windows 使用 `python` 和 `target\release\log-print.exe`。源文件、配置中的业务路径相对启动工作目录；`bin` 中带相对目录的可执行路径相对配置文件。完整参数见 [CLI](doc/cli.md)。

`read --wait-ms 1000` 在当前页暂时为空时最多等待1秒；到期仍如实返回空内容，持续订阅由Output负责。

`run` 前台运行并在 Ctrl-C 后收尾；`start` 后台运行，实例默认状态文件为 `.log-print/state.json`，输出在同目录的 `state.json.stdout.log`，诊断在 `state.json.stderr.log`。指定 `--state` 可选择另一个独立实例。每个实例仍只有一个 Core。

## 能力与插件

| 插件 | 能力与文档 |
|---|---|
| input-program | [任何语言程序的 stdout/stderr 分流采集](plugins/inputs/input-program/README.md)，不等换行 |
| input-file | [追加、截断、替换和段变更反馈](plugins/inputs/input-file/README.md) |
| input-serial | [跨平台串口只读接收](plugins/inputs/input-serial/README.md)，无 TX；设备与驱动覆盖以实测为准 |
| input-replay | [原始负载透明回放](plugins/inputs/input-replay/README.md)，源时间或明确的模拟节奏 |
| output-raw | [原始字节到 stdout 或文件](plugins/outputs/output-raw/README.md)，单/多流订阅 |
| output-transform | [编码、进制、分行、增删与派生发布](plugins/outputs/output-transform/README.md) |
| output-tui | [独立终端曲线、session、动态显示和 PNG/SVG 导出](plugins/outputs/output-tui/README.md) |
| output-webui | [本地离线 Web 图表、session、动态显示和 PNG/SVG 导出](plugins/outputs/output-webui/README.md) |

TUI/WebUI 各有独立 session。`session list/get/create/select/set/export` 通过 CLI 修改相应运行视图，不重启采集。WebUI 默认只监听 loopback，内嵌 ECharts 和字体，不依赖 CDN。

## 数据边界

- 默认只保留每流近期有界缓冲，**不保存历史**。启用插件或流的 `save.enabled` 后由 Core 事务保存，详见 [配置与保存](doc/configuration.md)。
- 统一读取返回 epoch、seq、head、next、可用范围和缺口。已保存数据可在重启后读取；未保存数据被覆盖后不能补回，Core 重启改变其 epoch。
- 保存成功在 SQLite 持久提交后确认。写入失败立即报错并阻塞该保存流，独立流继续；修复后用 `call resume --json '{"stream":"logs"}'` 手动恢复。不自动删除旧历史或降级为不保存。
- 原始与派生流独立。编码自动识别不确定时保留字节；未知缺失量明确报告，不声称硬件永不丢数据。无自动重启看门狗。

## 开发和验收

```sh
python3 tests/run.py
```

统一入口执行格式、Clippy、Rust 测试、构建和真实进程验收。自托管 Mac 和 GitHub Actions 的平台边界见 [CI](ci/README.md)；[性能报告](doc/performance.md)记录测法、原始结果和对照限制。

- [架构](doc/architecture.md)、[协议](doc/ipc-protocol.md)、[SDK 接入](doc/sdk.md)、[实施决定](doc/decisions.md)
- [贡献流程](CONTRIBUTING.md)、[文档入口](doc/README.md)、[简易 Agent Skill](skills/log-print/SKILL.md)
- MVP v1/v2 实际工作区和阶段文档留在被忽略的 `local-archive/`；正式版本保留原 Git 历史，历史中仍含 MVP 骨架。[归档记录](doc/implementation-progress.md)

本项目代码采用 MIT；第三方资产按各自许可证分发，见 [第三方说明](THIRD_PARTY.md)。
