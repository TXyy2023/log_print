# log_print

**为 AI Agent 和开发者打造的日志采集与归档工具。**

[![CI](https://github.com/TXyy2023/log_print/actions/workflows/validate.yml/badge.svg)](https://github.com/TXyy2023/log_print/actions/workflows/validate.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.92%2B-orange.svg)](Cargo.toml)

0.1.0 聚焦程序输出、日志文件、回放、转换与归档。通过 CLI 管理一个 Core 和独立插件，保留原始字节及可追踪的派生流。

[快速开始](#快速开始) · [文件与 SQLite 归档](plugins/outputs/output-file/README.md) · [参与贡献](CONTRIBUTING.md)

## 0.1.0 范围

- 输入：`input-program`、`input-file`、`input-replay`。
- 输出：`output-raw`、`output-transform`、`output-file`。
- Core 按流提供缓冲、可选持久保存及统一读取；插件使用 Rust SDK 和 `log-print/1` 协议。
- `output-file` 支持原始文件、JSONL、SQLite 及双目标，分别记录持久进度，支持验证后的恢复。
- 串口、TUI、WebUI 暂缓，不参与本版本默认构建和验收。
- 主程序和插件运行不依赖 Python 或 Node.js；Python 用于部分示例和测试。

## 快速开始

### 构建

需要 Rust 1.92 或更新的兼容稳定工具链。以下示例还需要 Python 3。

```sh
git clone --branch ver-0.1.0 https://github.com/TXyy2023/log_print.git
cd log_print
cargo build --release --locked --workspace
```

主程序和官方插件生成在 `target/release/`。以下命令均在仓库根目录运行。

### 采集第一条日志

创建日志文件，使用内置配置启动后台采集：

```sh
python3 -c "from pathlib import Path; Path('example.log').touch()"
./target/release/log-print start --config examples/basic.json
```

向文件追加一条日志，再通过 CLI 读取：

```sh
python3 -c "open('example.log','ab').write(b'temperature=23.5\n')"
./target/release/log-print read logs --raw --wait-ms 1000
```

输出：

```text
temperature=23.5
```

查看状态或停止采集：

```sh
./target/release/log-print status
./target/release/log-print stop
```

`--wait-ms 1000` 表示当前没有数据时最多等待 1 秒。需要前台运行时，使用 `run` 替代 `start`，通过 Ctrl-C 停止。

> Windows 下使用 `python` 和 `target\release\log-print.exe` 替换上述命令中的 `python3` 与可执行文件路径。

## 文件与 SQLite 归档

参见 [output-file 使用说明](plugins/outputs/output-file/README.md)，包含文件、SQLite 和双目标配置、恢复操作及持久性边界。

Core 保存与插件归档是两个独立状态。需要插件离线后补齐时，应开启对应 Core 流的保存；已被缓冲覆盖的未保存数据无法恢复。双目标没有跨文件和数据库的原子事务。

## 插件与验证

各插件目录中的 README 说明配置；[Rust SDK](crates/log-plugin-sdk) 和 [协议类型](crates/log-proto/src/lib.rs) 随代码发布。开发与测试说明见 [贡献指南](CONTRIBUTING.md) 和 [CI 说明](ci/README.md)。

```sh
python3 tests/run.py
```

测试会构建当前 workspace，并运行真实 Core、插件和归档恢复场景。实际通过平台以当前提交的 CI 结果为准；进程中断测试不等于真实断电或坏盘验证。

`doc/` 默认仅保存在本地，待选择具体文档后再加入跟踪白名单。
