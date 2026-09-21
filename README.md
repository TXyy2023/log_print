# log_print

为开发者和 AI Agent 采集、读取、转换和保存日志的本地工具。当前版本 **0.1.2**，通信契约 **log-print/2**。

四个核心部件负责 CLI 运行组织、内存转发、协议通信和插件接入。两个输入插件采集文件或程序/tmux 输出；三个输出插件负责终端显示、保存以及转换后发布派生流。Core 不保存日志，默认 TCP，可选 UDP。

## 开始使用

```sh
cargo build --release --locked --workspace
./target/release/log-print --help
```

需要 Rust 1.92 或更新的兼容稳定工具链。阅读 [安装与构建](doc/public/installation.md)、[快速开始](doc/public/quickstart.md) 和 [使用手册](doc/public/index.md)。

内存缓冲有界，满时覆盖最早数据。Input 发布不等待 Output 消费；发布成功不代表下游已处理或保存。Core 重启会失去全部缓冲，旧 `log-print/1` 配置和客户端需要迁移。

## 开发与验证

```sh
python3 quality/run.py
```

[实施与验收清单](quality/release-0.1.2.md) · [贡献指南](CONTRIBUTING.md) · [测试说明](quality/README.md) · [MIT 许可证](LICENSE)

仓库分为 [project](project/README.md)、[doc](doc/README.md) 和 [quality](quality/README.md)。公开说明位于 `doc/public/`；本地文档站入口见 [文档站](doc/site/README.md)。
