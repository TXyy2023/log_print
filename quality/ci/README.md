# 正式验证入口

macOS、Linux、Windows 均从仓库根运行 `python3 quality/run.py`（Windows 可用 `python`）。要求 Python 3.12+、Rust stable、rustfmt、Clippy。

入口执行格式、workspace Clippy、Rust测试、构建，以及 `quality/tests/v2/` 的真实进程验收。构建失败后不接受旧二进制；任何必需脚本缺失都失败。旧 log-print/1 测试已保留于 `quality/archive/v1/`，其存储/回放保证不能套用本版。

报告位于 `quality/artifacts/validation/<UTC>-<系统>-<PID>/`，记录实际工具链、命令、返回码、耗时和完整日志。`--report-dir PATH` 指定新目录；`--list` 只列命令；`--skip-build` 仅用于迭代，报告明确标不完整。每步超时默认 1800 秒，supervisor阶段最多120秒。

子测试使用 UTF-8。CI包装器在Windows将CTRL_BREAK转换为KeyboardInterrupt，使finally清理自建进程。超时先中断测试进程组，再有界终止；不按名称清理用户进程。CLI测试使用实际PIPE，检查后台启动不会因继承句柄使调用方永久等待。

tmux验收创建唯一socket名、独立服务/窗格，只清理自建资源；Unix无tmux或Windows不支持时明确skip。GitHub Linux/macOS任务安装tmux后验收，Windows标注不适用。跨平台通过必须引用各自报告，单机通过不能代替。

GitHub `validate.yml` 在push/手动运行三系统矩阵。`self-hosted.yml` 仍仅手动main分支且仓库变量启用时运行已有runner，不注册、不创建定时任务。可本机使用 `LOG_PRINT_SELF_HOSTED_ENABLED=true bash quality/ci/self-hosted/run-macos.sh`。

本套件验证软件行为，不构成真实硬件、真实断电或无限负载保证。测试用合成字节与真实软件日志工作负载分开。
