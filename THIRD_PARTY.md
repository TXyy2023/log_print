# 第三方组件

Rust 依赖的实际版本和校验值由 Cargo.lock 锁定，可通过 `cargo metadata --locked --format-version 1` 取得完整依赖图。以下是直接影响架构和分发的组件；依赖自身的许可证保留在 crate 源码中。

| 组件 | 用途 | 许可证 |
|---|---|---|
| Tokio / Serde / serde_json / Clap / UUID | 运行时、配置、CLI、身份 | 各库 MIT 或 MIT/Apache-2.0，按锁定 crate 元数据 |
| rusqlite / SQLite | Core事务与历史 | rusqlite MIT；SQLite public domain |
| fs2 / sha2 | 独占锁、内容校验 | MIT/Apache-2.0 |
| process-wrap / chrono | 子进程组与Job、回放时间 | MIT/Apache-2.0 |
| serialport 4.10.0 | 串口 | MPL-2.0 |
| encoding_rs 0.8.40 | 字符编码 | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| chardetng / regex | 编码检测、文本规则 | MIT/Apache-2.0 |
| Ratatui / Crossterm / Axum / Plotters | TUI、Web服务、图表导出 | MIT |
| Apache ECharts 6.1.0 | 离线Web图表 | Apache-2.0；[LICENSE](plugins/outputs/output-webui/assets/ECHARTS-LICENSE)、[NOTICE](plugins/outputs/output-webui/assets/ECHARTS-NOTICE)、[来源](plugins/outputs/output-webui/assets/SOURCES.md) |
| Noto Sans SC | 中文导出字体 | SIL Open Font License 1.1；[许可证](crates/log-plot/assets/OFL.txt)、[来源](crates/log-plot/assets/SOURCES.md) |

性能对照程序不嵌入产品，其版本/来源和构建方式由性能报告单独列出。新增分发资产时同时记录来源、固定版本、许可证和NOTICE，不仅复制压缩脚本。
