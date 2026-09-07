# 正式版验证入口

在 macOS、Linux 或 Windows 使用 Python 3.12+、Rust stable（rustfmt/clippy）运行：

```sh
python3 tests/run.py
```

Windows 可使用 `python tests/run.py`。协调器共执行 10 个必做阶段：格式检查、整个 workspace 的 Clippy、Rust 测试、构建、真实 Core 协议测试、主进程/插件生命周期测试、独立可靠性回归（缺失保存段、损坏、反馈环、握手 RST 等），以及 `examples/io/verify.py`、`examples/io/languages.py`、`plugins/outputs/output-webui/tests/verify_ui.py`。POSIX UI 验证开启 PTY 渲染/缩放，Windows 使用 headless；所有指定脚本均为必做，缺失会失败。构建失败时跳过进程测试，避免误用旧二进制。

语言阶段运行实际程序，经 `input-program → Core → output-raw` 核验 stdout/stderr 字节及源程序退出前的无换行推送，报告为 `input-languages.json`。Python/Rust 必需；Node.js、C、C++、Go、shell 按实际可用工具运行，缺失明确记为 skipped，已安装但构建或执行失败则使阶段失败。完整套件通过不代表被跳过的语言已验证；逐平台支持范围以该 JSON 的实际版本与状态为准。

报告位于 `artifacts/validation/<UTC>-<系统>-<PID>/`，包含实际 Python 路径、Rust/Cargo 版本、平台、命令、返回码、时长及完整日志。`--report-dir PATH` 指定尚不存在的报告目录。`--list` 只列命令；`--skip-build` 供迭代复查，报告标记不完整，不能作为完整正式验收。每个命令默认超时 1800 秒，可用 `--timeout` 调整。超时或中断先通知自身测试进程组，让测试的 finally 回收它创建的实例；不按进程名称清理用户程序。

所有 Python 子测试显式使用 UTF-8，避免 Windows 本地编码破坏 Rust JSON 中的中文。`ci/python-test.py` 保持测试参数和相邻模块导入，并在 Windows 将 CTRL_BREAK 转为 KeyboardInterrupt，使测试能进入 finally 清理；它不会将测试错误改为成功。无法响应中断的进程仍由协调器在有界等待后终止，日志保留失败。

本地 Mac 优先使用同一入口的 shell 包装：

```sh
LOG_PRINT_SELF_HOSTED_ENABLED=true bash ci/self-hosted/run-macos.sh
```

包装不启动 login shell、不改 PATH、不安装依赖或注册 runner，可通过 `LOG_PRINT_CI_PYTHON` 指定现有 Python 路径。`--list` 同样适用。若当前 Python 低于 3.12，入口明确失败，不静默换系统 Python。这里只运行手动验收，不创建定时任务或监控。

云工作流 `validate.yml` 在 push 或手动调度时运行 `ubuntu-latest`、`macos-latest`、`windows-latest`，Python 固定为 3.12，Rust 使用 stable 并记录实际版本；Cargo.lock 用 `--locked` 保持依赖版本。各平台分别上传报告，单个平台的成功不能代表其他平台。

`self-hosted.yml` 仅允许手动 main 分支运行，且仓库变量 `LOG_PRINT_SELF_HOSTED_ENABLED=true` 才调度已有标签 `[self-hosted, macOS, ARM64, log-print-ci]` 的 runner。不触发 PR 或定时任务，不注册 runner；需要预先安装 Python/Rust 及足够新的 GitHub Actions runner。当前文件的存在不表示 runner 已注册或线上任务已成功。

动作版本参考 [checkout 官方用法](https://github.com/actions/checkout)、[setup-python 官方用法](https://github.com/actions/setup-python)、[upload-artifact 官方用法](https://github.com/actions/upload-artifact) 和 [Rust toolchain action](https://github.com/dtolnay/rust-toolchain)。原生串口依赖关闭默认功能，当前配置不要求 Linux libudev；若将来启用枚举等新依赖，须同步平台准备步骤。

协议/进程测试、无界面绘图、人工终端/浏览器视觉验收和真实硬件验收分别记录。此入口本身不构成真实设备、断电恢复、性能或跨平台通过证明。
