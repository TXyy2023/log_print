# 正式验证入口

macOS、Linux、Windows 均从仓库根运行 `cargo run --locked -p log-print-quality --`。要求 Rust stable、rustfmt、Clippy、Node.js 22.12+ 与 npm。Node 仅在构建时使用，发布后的 WebUI 二进制包含本地资源。

入口先由 Cargo 构建 Rust 验证工具，再执行格式、workspace Clippy、Rust测试、产品二进制及 example 构建，以及 `quality/src/suites/` 的真实进程验收。产品构建排除正在运行的 quality 工具，避免 Windows 可执行文件锁冲突。构建失败后不接受旧二进制；任何必需脚本缺失都失败。旧 log-print/1 测试已保留于 `quality/archive/v1/`，其存储/回放保证不能套用本版。

报告位于 `quality/artifacts/validation/<Unix毫秒>-<系统>-<PID>/`，记录实际工具链、命令、返回码、耗时和完整日志。`--report-dir PATH` 指定新目录；`--list` 只列命令；`--skip-build` 仅用于迭代，报告明确标不完整。每步超时默认 1800 秒，supervisor阶段最多120秒。

子测试为 Rust 原生进程，UTF-8 JSON 协议与报告跨平台一致。中断信号由 Rust 处理器通知测试，在有界等待后通过 Drop 清理自建进程。超时先中断测试进程组，再有界终止；不按名称清理用户进程。CLI测试使用实际PIPE，检查后台启动不会因继承句柄使调用方永久等待。

tmux验收创建唯一socket名、独立服务/窗格，只清理自建资源；Unix无tmux或Windows不支持时明确skip。GitHub Linux/macOS任务安装tmux后验收，Windows标注不适用。跨平台通过必须引用各自报告，单机通过不能代替。

GitHub `validate.yml` 在push/手动运行三系统矩阵。`self-hosted.yml` 仍仅手动main分支且仓库变量启用时运行已有runner，不注册、不创建定时任务。可本机使用 `LOG_PRINT_SELF_HOSTED_ENABLED=true bash quality/ci/self-hosted/run-macos.sh`。

`validate.yml` 每次使用包含 run ID / attempt 的新报告目录。`browser-readiness.mjs` 只接受本提交完整入口生成的最终报告，要求 `frontend` 和 `build` 均成功；报告缺失、损坏、不完整或 SHA 不符时失败关闭。运行时套件失败仍执行浏览器验收，原失败继续使 job 失败；构建失败明确说明浏览器跳过原因。Chromium 安装和浏览器本身失败也不会被忽略，矩阵仍为 `fail-fast: false`。此门控不改变 `self-hosted.yml`。用 `node --test quality/ci/browser-readiness.test.mjs` 检查正常、构建失败、运行时失败和报告故障分支。

归档缺口回归使用 quality 专用 Core 夹具：`log-proto/test-support` 编译特性提供显式 TCP 构造入口，正常监听器不调用它，也不会因环境变量暂停。订阅应答和记录 1 均经真实 TCP 发出；归档确认 checkpoint=2 后，Core 对记录 1 的发送完成确认仍被屏障暂扣，尚未读取下一批。发布 2..5 后先确认 Core oldest=head=5，再释放屏障；真实 SDK / output-file 必须产生唯一缺口 2..4、保存记录 1/5/6/7、推进 checkpoint=8，历史行与曲线均保留缺口及后续上下文。使用内部缺口保留原先的行解码器验收范围；首条记录之前的缺口显示不由本用例作保证。

该屏障不持 Core/stream 锁、不伪造 Gap，不靠 TCP 缓冲大小或 sleep 制造丢失。释放文件单调存在，等待有 30 秒上限；父进程 EOF 可立即取消，并有单独回归。小记录快速发布、小记录带间隔、大记录加归档延迟三种配置都执行精确断言，WebUI/TUI 共用同一场景。日志打印起点、实际保留范围、缺失序号、记录序列与 checkpoint，便于跨平台定位。

本套件验证软件行为，不构成真实硬件、真实断电或无限负载保证。测试用合成字节与真实软件日志工作负载分开。
