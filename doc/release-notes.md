# log_print 1.0.0

2026-09-08，第一版正式实现，协议`log-print/1`。单个主进程管理唯一独立Core和多个独立插件；CLI、Rust SDK与语言无关协议均可接入。当前版本不承诺后续协议不变，不兼容升级会明确拒绝旧连接。

本次交付：

- 程序stdout/stderr、文件跟随、只读串口和透明Replay；输入不等待完整行。多语言程序复用通用管道Input。
- Core按流提供有界缓冲、可选SQLite分段保存、统一历史/实时读取、epoch/seq/缺口与身份权限；持久失败阻塞受影响流，修复后手动恢复。
- 原样字节输出、保守编码识别/手动覆盖、进制、分行和内容编辑；派生流保留原始数据，经Core组合并拒绝反馈环。
- 独立Ratatui TUI与本地ECharts WebUI；多曲线、独立session、CLI动态显示、revision冲突检查及PNG/SVG导出。图表窗口有界，原始交付与绘图刷新分离。
- 启动/停止/重启、状态/配置、流读取、session选择/修改/导出的CLI；可选限时读取适配短暂空页。Windows后台CLI处理调用者标准句柄继承，验证真实PIPE EOF语义。
- 统一10阶段验收、自托管Mac入口、三平台GitHub Actions、完整教程/SDK说明/简单Agent Skill、端到端及资源实测证据。

[验证与平台边界](validation.md)、[性能与对照](performance.md)、[持续UI与浏览器资源](../plugins/outputs/output-webui/SUSTAINED.md)分别说明实际证据；不会把配置刷新频率当实测FPS、把PTY当物理串口或可见终端、把短时测量当任意规模的稳定上界。浏览器RSS在三分钟测量中仍有增长，需要更长时间及堆分析才能判断原因。

MVP v1/v2的实际源码、嵌套Git和阶段文档完整保存在本地`local-archive/`，不进入新的Git提交。原始`doc/todolist.md`字节未改；原初始化历史`66d5e51`保留，没有清洗或强推。[归档与交付记录](implementation-progress.md)。

运行从根[README](../README.md)开始。正式仓库为[TXyy2023/log_print](https://github.com/TXyy2023/log_print)；具体被测提交和CI结果见验证报告，而非以最后文档提交替代被测二进制版本。
