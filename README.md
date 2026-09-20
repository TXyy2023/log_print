# log_print

为开发者和 AI Agent 采集、读取、转换与归档日志的本地工具。当前版本 **0.1.1**。

输入支持程序 stdout/stderr、普通日志文件和文件回放；输出支持原始字节、派生转换，以及文件、JSONL、SQLite 和双目标归档。串口、TUI、WebUI 暂缓，不参与默认构建。

## 开始使用

需要 Rust 1.92 或更新的兼容稳定工具链：

```sh
cargo build --release --locked --workspace
./target/release/log-print --help
```

完整步骤见 [安装与构建](doc/public/installation.md) 和 [快速开始](doc/public/quickstart.md)。[使用手册](doc/public/index.md) 按采集、读取、转换、归档等任务组织，参数见配置与插件参考。

Core 保存与输出归档是独立状态。需要恢复归档时，应保留源历史和所有归档状态，详见 [恢复与完整性](doc/public/guides/recovery.md)。

## 参与开发

[贡献指南](CONTRIBUTING.md) · [CI 说明](quality/ci/README.md) · [Rust SDK](project/crates/log-plugin-sdk) · [MIT 许可证](LICENSE)

```sh
python3 quality/run.py
```

文档正文统一维护在 `doc/public/`；README 只提供仓库入口。本地文档站运行方法见 [文档站](doc/site/README.md)。

## 仓库目录

- [project/](project/README.md)：主程序、公共库、插件、使用示例和外部真实软件工作负载。
- [doc/](doc/README.md)：公开文档、内部资料和文档站工具。
- [quality/](quality/README.md)：独立测试、性能测试、CI 和本地验收产物。

Cargo 配置和 GitHub 工作流入口保留在仓库根目录及 `.github/workflows/`；构建命令仍在仓库根目录运行。
