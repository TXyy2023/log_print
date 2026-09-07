# 正式版本验收记录

2026-09-08。本版独立重跑验收，不计入任何MVP旧结果。最终源码提交`a0be66806d16f70032b9f868f615bb2f0001e0c7`的[三平台GitHub Actions](https://github.com/TXyy2023/log_print/actions/runs/34150338258)全部通过，每个平台均完成10个阶段。阶段通过仍包含下表明确列出的条件跳过项；后续交付提交仅更新文档和证据。

## 平台矩阵

| 平台 / 环境 | 已完成结果 | 边界 |
|---|---|---|
| macOS 26.5 ARM64 / Rust1.92 / Python3.14.1 | 本地self-hosted入口10/10阶段通过，源码e0b9203；5279e8d的同义Core循环改写后11组协议通过；7e81460的诊断增强后5组supervisor通过 | 串口PTY的驱动配置报ENOTTY，明确skip；物理串口未测 |
| Linux ARM64 / Apple container / Rust1.92 / Python3.12.14 | 5279e8d完整10/10阶段通过；串口PTY及文件启动竞态均通过 | 实际Python/Rust/C/C++/sh五种语言通过，容器无Node/Go，两项明确skip；无Linux桌面视觉/物理串口验收 |
| GitHub Ubuntu x86_64 / Rust1.98.1 / Python3.12 | a0be668完整10/10阶段通过；7种语言、串口PTY与6组UI通过 | 无实际Linux桌面或物理串口验收 |
| GitHub macOS ARM64 / Rust1.98.1 / Python3.12 | a0be668完整10/10阶段通过；6种语言与6组PTY UI通过 | Go工具缺失跳过；串口PTY报ENOTTY跳过；无物理串口验收 |
| GitHub Windows AMD64 / Rust1.98.1 / Python3.12 | a0be668完整10/10阶段通过；真实PIPE EOF、原生文件替换、7种语言和5组headless UI通过 | 不运行POSIX权限/PPID/信号故障注入与串口PTY；ACL内容未独立核验，headless不等于可见console验收 |

可克隆的[平台和教程摘要](validation-evidence/README.md)保留各自版本、环境、返回码、跳过项和源报告哈希。完整本地结果位于`artifacts/validation/formal-macos-e0b9203/`、`artifacts/linux/validation/formal-linux-5279e8d/`，最终云CI原报告位于`artifacts/ci/34150338258-{ubuntu,macos,windows}/`及对应Actions附件；它们是验收产物，不属于用户运行状态。

## 实际验证内容

| 范围 | 结果与含义 |
|---|---|
| 归档 | 实施前7,487文件/链接逐项核验；8类源码、嵌套Git与文档抽样恢复后哈希一致；原始todo未改 |
| Rust测试 | 20项：SDK受限双向传输、数值/session/export、转换和信号处理等；完整workspace格式、Clippy及构建均执行 |
| 主程序/CLI | 最终POSIX 6组、Windows 4组：已知原始字节、配置与身份、插件启停/重启、正常回收、半途启动失败、限时读取、真实PIPE EOF；POSIX额外核对直接父子PID/权限，并注入启动中断/Core死亡，Windows明确不执行这些条件项 |
| Core协议 | 11组：版本/权限/epoch、字节、多流/派生、覆盖、历史转live、慢端隔离、持久重启/幂等、容量、不可写与恢复、hot journal进程崩溃恢复、控制隔离、超长帧 |
| 独立可靠性审查 | 6组：保存尾段/全部段缺失、记录/校验损坏、隐藏反馈环、RST握手身份释放、源时间参与幂等 |
| 官方IO | Linux7/7；Mac6通过/1PTY跳过；Windows的适用场景全部通过，串口PTY不运行。包括stdout/stderr无换行二进制、文件追加/截断/替换、注册前固定tail位置、Replay透明负载、跨块转换和程序后代清理 |
| 多语言输入 | Mac七种已有语言运行时全部真实运行，经input-program→Core→raw逐字节核对；NUL、0xff及无换行首段必须在源退出前到达。[版本与矩阵](input-coverage.md) |
| UI自动化 | POSIX 6组；Windows 5组headless，跳过可见终端/PTY。独立TUI/WebUI同流、拆块数字、SSE增长、session隔离/revision冲突、四图导出与元数据、本地资产；POSIX额外验证PTY resize/选择 |
| 实际浏览器 | Chrome实际观察连续双曲线、CLI配置可见、其他session隔离、700×900布局、暂停与session选择；PNG实际查看后修正字体与网格。[图像和记录](../plugins/outputs/output-webui/VERIFICATION.md) |
| 干净目录教程 | README与Skill两套release流程均精确读出17字节`temperature=23.5\n`，四个进程及state/runtime目录清理完成；dashboard完成双曲线数据、revision1→2、PNG导出及停止。[摘要](validation-evidence/README.md) |
| 性能 | 两轮×6场景，各12,000条逐字节通过；主轮raw/save/TUI中位延迟0.315/1.024/0.330ms。[完整尾延迟、资源和对照限制](performance.md) |
| 持续UI | 180秒、1000行/秒，每插件2session×2曲线匹配180000行，8次修改隔离通过；浏览器进程树单列。RSS仍有增长，未证明长时稳定平台。[原始证据](../plugins/outputs/output-webui/SUSTAINED.md) |

运行入口为`python3 tests/run.py`，Mac包装为`LOG_PRINT_SELF_HOSTED_ENABLED=true bash ci/self-hosted/run-macos.sh`。10阶段依次覆盖fmt、Clippy、Rust测试、构建、协议、生命周期、独立可靠性、IO、语言、UI。局部迭代结果不冒充一次新的完整套件；每份报告保留真正被测SHA或二进制哈希。

## 发现并修复的问题

- 独立故障审查修复了保存段缺失误认空历史、隐藏处理反馈环和RST连接身份残留；用失败样例形成回归。
- Linux后台TUI原先读取进程控制终端尺寸，已改为读取指定TTY；修复后PTY与完整套件通过。
- 文件tail初始化原先晚于注册，可能错过启动后追加；新增代理延缓注册的确定性回归证明旧版失败、新版通过。
- 无控制台时Ctrl-C注册失败不再冒充退出信号；控制通道仍承担正常停止。Windows实际结果单独列出。
- CI首轮暴露新Rust1.98 Clippy写法要求、测试缺少sys导入及协议观察器无缓冲逐字节读的开销；均已修正，不降低既有内容、隔离或时间断言。
- Windows后台子进程继承调用者的标准句柄，使CLI虽退出但PIPE不能到达EOF。主程序在启动任何子进程前清除三个调用者标准句柄的继承位，子进程仍使用显式日志或NUL输出。f5a3a23真实Windows CI中，start/status/read/stop全部退出且两个PIPE都到达EOF，后台生命周期与原UI脚本均通过；文件捕获和只读PID观察作为诊断补充。
- Windows可靠性夹具原先只结束SQLite事务而未关闭连接，导致临时文件无法删除；现显式提交并关闭连接。文件替换夹具则改用支持替换打开目标的Rust原生rename，持续保持采集并检查完整字节和段变更；a0be668三平台CI均通过，Python旧替换API的限制单独记录，详见[输入覆盖](input-coverage.md)。

## 未验证边界

物理串口设备/驱动、实际掉电或坏盘、完整回滚全部历史元数据、源程序主动脱离进程组的守护进程、任意规模或小时级稳定性均未覆盖。UI的100ms是刷新配置，CLI往返不是采集到屏幕扫描延迟，没有宣称精确FPS。

可见macOS Terminal尚未人工验收：内置Computer Use安全规则拒绝访问`com.apple.Terminal`，未绕过。POSIX PTY有真实渲染和控制证据，不能代替该应用的可见窗口验证。串口始终只读，没有发送硬件TX。
