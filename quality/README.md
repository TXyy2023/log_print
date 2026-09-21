# 0.1.2 测试与 CI

从仓库根目录运行：

```sh
python3 quality/run.py
python3 quality/run.py --list
```

需要 Python 3.12+、Rust stable（rustfmt / Clippy）。入口执行格式、Clippy、全部 Rust 测试、构建，以及真实进程协议、CLI 生命周期、输入和输出验收。测试生成的数据均为明确标注的 fixture，不冒充真实软件日志。

| 位置 | 验收范围 |
| --- | --- |
| 各 crate 的 Rust 测试 | 协议限额、权限/缓冲、SDK、输入配置、输出文件/SQLite事务及转换窗口 |
| `tests/v2/protocol.py` | TCP/UDP、独立多流/多订阅、覆盖、慢消费者、非法身份和Core重启 |
| `tests/v2/supervisor.py` | CLI真实管道、配置快照、后启动/重启、UUID绑定、拒绝旧配置及清理 |
| `tests/v2/inputs.py` | 静态文件、跟随/截断/替换、双通道程序、异常退出、进程树、隔离tmux |
| `tests/v2/outputs.py` | 显示与保存读回、转换派生流、停止与错误 |
| `tests/docs/verify_links.py` | 文档站链接与附件 |
| `archive/v1/` | 旧组件/旧协议验收，保留溯源、不作为本版通过证据 |
| `artifacts/` | 本地忽略的报告、完整日志及备份 |

`quality/ci/README.md` 说明报告与跨平台边界。tmux 仅 Unix 环境且已安装时运行，缺失必须记录 skip；本机实测结果单独记录。构建失败不能接受旧二进制，`--skip-build` 仅迭代，不能替代完整检查。

文档：

```sh
npm run build:local --prefix doc/site
npm run build:public --prefix doc/site
python3 quality/tests/docs/verify_links.py doc/site/dist/local
python3 quality/tests/docs/verify_links.py doc/site/dist/public
```

[0.1.2 实施与验收记录](release-0.1.2.md)
