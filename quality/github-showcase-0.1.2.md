# 0.1.2 GitHub 展示与 Skill 验证

日期：2026-09-22。产品代码基线 `7a3ab38d4651c3c44be60efac2ccac613172a2f8`，本轮不改变协议、Cargo 版本和运行代码；新增首页展示、实际演示录像、可独立安装的 Skill，并修正文档入口。

## 变更范围

- README：原创标题图、CI/Rust/版本/协议/平台/MIT 徽章、功能范围、可复制快速开始、视频、Agent 安装与贡献入口。
- 两段实际 CLI 演示：静态文件读取，程序双通道采集/派生编号/JSONL 与 SQLite 保存；脚本驱动，教学输入明确标注。
- Skill：检查 CLI 版本/帮助及整套 Core/插件；缺失时给出源码构建方式；提供实例、真实 UUID、有限读取、归档确认、归属停止与日志数据边界。
- 公开文档：演示页与素材进入显式构建白名单；修正旧测试入口和公开 checkout 中不存在的本地文档链接。内部模块资料与归档继续仅本地保留。

## 本地验证

环境：macOS 26.5 arm64、Rust/Cargo 1.92.0、Python 3.14.1。

- `cargo build --release --locked --workspace`：通过，构建整套实际 release 程序。
- `python3 quality/run.py`：9 个阶段全部通过，含格式、Clippy、64 个 Rust 测试、构建与 48 个真实进程场景。包含隔离 tmux 用例。
- Skill `quick_validate.py`、许可证一致性、Skills CLI 自动发现与隔离安装：通过，安装包含完整四文件。
- Skill 7 项行为验证：版本/帮助与子程序存在、安装完整性、启动/真实 UUID、文件跟随与有限空读、JSON/raw 快照语义、UUID 绑定显示及 JSONL/SQLite 确认、正常停止和状态清理。
- 公开文档构建与隔离审查：28 个白名单输入、157 个输出通过；24 页、836 个本地目标检查零错误。README、公开正文、Skill 和演示说明的 83 个相对文件引用全部存在。
- 浏览器检查：标题图、6 个徽章、2 个视频封面均加载成功，无横向溢出；两段 MP4 均实际播放，1600×900、无媒体错误，公开演示页无控制台错误。
- 两段录像：18 项实际运行断言通过，含逐字节读取、独立派生流、JSONL/SQLite 读回和 SQLite integrity_check；只清理自建实例。
- README 快速开始：直接提取 Markdown 原样 shell 命令，在临时目录中运行 release 程序；准确读到 `temperature=23.5\n`，Core/来源插件正常停止且未强制终止，状态文件移除。

录像的逐项断言、实际命令输出、媒体编码信息在 [demo evidence](demos/README.md) 中公开；只有已脱敏的教学证据进入仓库。未公开的完整测试与安装日志保留于 `quality/artifacts/github-showcase/`。

推送后的 CI 以 [Formal validation](https://github.com/TXyy2023/log_print/actions/workflows/validate.yml) 对对应提交的实际结果为准。上面的本地结果不替代其他平台、长时间稳定性或真实设备验收；上一版跨平台发布结果见 [0.1.2 验收](release-0.1.2.md)。

## 发布边界

原 `ver-0.1.2` 标签及 Release 保留，不移动标签。本轮是基于相同产品版本的展示与 Skill 更新，安装最新 Skill 请使用默认分支。各技能目录的索引或审核与仓库提交分别核实；未获得公开条目或收录回执时不宣称上线。MIT 许可证保持不变。
