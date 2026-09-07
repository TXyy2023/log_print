# 正式版本验收记录

2026-09-08，平台收口中。macOS与Linux完整套件已通过；Windows云CI结果仍待执行。MVP 的测试结果不计入本表。

| 范围 | 当前证据 |
|---|---|
| 归档 | 实施前7,487文件/链接逐项核验；8类源文件、嵌套Git与文档抽样独立恢复后哈希一致；原始todo未改 |
| 主程序/CLI | 真实Core及插件的直接父子PID、原始字节、配置/令牌、启停重启、正常回收、半途启动失败、Core死亡、启动中SIGINT已有5组通过 |
| Core协议 | 已有11组真实Core进程测试通过（新增真实进程异常退出留下SQLite hot journal后的恢复），含版本/权限、任意字节、派生、缓冲覆盖、持续发布中历史转live、慢端隔离、保存重启/幂等、容量、权限故障与恢复、控制隔离 |
| 独立可靠性审查 | 已修复并通过6组：尾段/全部段缺失、内部行或哈希损坏、隐藏处理环、RST握手身份释放、源时间参与幂等 |
| SDK | 5项受限双向传输回归通过：取消不截帧、总deadline、满控制队列、双向拥塞故障清理、Drop关闭；有效回复优先于随后EOF |
| 输入/转换 | 真实程序stdout/stderr、无换行二进制、文件追加/截断/替换、透明replay与跨块UTF8/末行、程序后代清理已有通过；各算法单元测试随完整套件汇总 |
| UI自动化 | 两个独立UI同流、拆块数字、SSE增长、session隔离与revision冲突、四图导出/元数据、PTY resize/选择、离线资产已有通过 |
| 实际浏览器 | Chrome中连续双曲线、CLI配置及时可见、其他session隔离、700×900布局、暂停和session下拉切换；PNG实际查看后修正字体与网格 |
| 可见终端窗口 | 未验证。内置Computer Use安全规则拒绝打开macOS Terminal；未绕过。POSIX PTY渲染与控制已测，不能代替可见终端验收 |
| 物理串口 | 未提供适合的真实测试设备，未发送TX；macOS PTY在驱动配置时报ENOTTY，明确skip，不计物理串口通过 |
| 性能 | 两整轮×6场景，每场景12,000条逐字节通过；主报告为第二整轮，raw/save/TUI中位延迟0.315/1.024/0.330ms，完整资源、尾延迟和基线限制见performance.md |
| macOS完整套件 | 早期统一入口9个阶段全部通过；新增语言/启动竞态/CLI等待后正在复跑最终10阶段，最终结果见下方 |
| Linux ARM64完整套件 | Apple container中Debian/Rust1.92/Python3.12.14，9阶段全部通过，IO6组含串口PTY；另headless UI5组通过。无Linux桌面视觉或物理硬件验收 |
| Windows云CI | 待实际执行，不由其他平台结果推断 |

运行入口：`python3 tests/run.py`。详细命令、工具链、机器、退出码和完整日志写入本地 `artifacts/validation/`。完整套件包含格式、Clippy、Rust测试、正式构建、协议、生命周期、独立可靠性、IO与UI进程脚本。单次代码修复后的局部通过不代替最终整套结果。

未覆盖：实际断电/坏盘、所有串口设备/驱动、恶意修改或完整回滚所有历史元数据、源程序主动脱离进程组的守护进程、任意规模与长时间稳定性。观察到的未知数据量保持未知，不写成零。


本地完整macOS报告：`artifacts/validation/20260907T171209Z-darwin-15616/results.json`（UTC目录时间；本地日期2026-09-08）。首次整套仅格式失败，修正后重新执行九阶段全过。热日志恢复是进程崩溃注入，不等于断电测试。

本地Linux报告：`artifacts/linux/validation/formal-linux-final-7e9c/results.json`。首次PTY resize暴露Ratatui查询错误终端，修复为读取指定TTY尺寸后整套通过；独立容器已停止。

教程验收：README与Skill均在独立空目录拷贝release二进制和basic配置后运行，精确读出17字节`temperature=23.5\n`，每轮4个进程及state/runtime目录均已回收。`--wait-ms`实际覆盖文件轮询尚未采到数据的空窗；dashboard示例也在临时目录完成双曲线数据、revision1→2、PNG导出及清理。证据为本地`artifacts/quickstart.json`与`artifacts/dashboard-quickstart.json`。

持续UI：每秒1000行、180秒、每插件2session×2曲线均匹配180000行，8次CLI修改与隔离通过；外部Chrome资源单列，RSS仍呈增长，未证明长时稳定平台。[完整记录](../plugins/outputs/output-webui/SUSTAINED.md)。
