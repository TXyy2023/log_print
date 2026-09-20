# 测试与 CI

从仓库根目录运行统一验收：

```sh
python3 quality/run.py
# 只查看检查命令，不执行测试
python3 quality/run.py --list
```

完整验收包含格式、Clippy、Rust 单元测试、构建，以及协议、主进程、可靠性、I/O、语言输入和 output-file 归档检查。参数及平台说明见 [CI 说明](ci/README.md)。

| 目录 | 内容 |
| --- | --- |
| `tests/protocol.py` | Core 协议验收及共享协议测试工具 |
| `tests/app/` | 主程序生命周期与可靠性测试 |
| `tests/io/` | 输入输出集成与语言接入测试 |
| `tests/output-file/` | 文件和 SQLite 归档验收 |
| `tests/docs/` | 文档站生成链接与附件检查 |
| `benchmarks/` | 单独运行的性能测试，不算作默认验收 |
| `ci/` | 跨平台子测试包装器、自托管 CI 入口 |
| `artifacts/` | 新旧验收报告、日志、备份和截图，本地忽略 |
| `archive/legacy-test/` | 历史测试资料，保留原貌且不自动执行 |

本地暂缓的 WebUI 测试位于 `tests/output-webui/`，继续被 Git 忽略，不参与当前版本默认验收。GitHub 工作流必须保留在根目录 `.github/workflows/`；实际检查由本目录入口执行。

文档构建与验收：

```sh
npm run build:local --prefix doc/site
npm run build:public --prefix doc/site
python3 quality/tests/docs/verify_links.py doc/site/dist/local
python3 quality/tests/docs/verify_links.py doc/site/dist/public
```

历史报告中的命令和路径是当时记录，不改写为新执行证据。源码和脚本的新位置记录在 `doc/site/repository-paths.json`，文档站通过映射解析旧链接。
