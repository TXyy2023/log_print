<span id="quick-start"></span>

# 快速开始

先按 [安装](installation.md) 构建，在仓库根目录运行。以下创建的是演示输入。

```sh
python3 -c "from pathlib import Path; Path('example.log').touch()"
./target/release/log-print --state .log-print/quickstart.json start --input-file source=./example.log \
  --output-raw screen --no-autostart screen
./target/release/log-print --state .log-print/quickstart.json streams
python3 -c "open('example.log','ab').write(b'temperature=23.5\n')"
```

`start` 和 `streams` 返回 Core 分配的 UUID，以及说明、归属、缓冲范围。把真实 UUID 填到下一条命令：

```sh
./target/release/log-print --state .log-print/quickstart.json read STREAM_UUID --wait-ms 1000
./target/release/log-print --state .log-print/quickstart.json plugin start screen --stream STREAM_UUID
./target/release/log-print --state .log-print/quickstart.json plugin stop screen
./target/release/log-print --state .log-print/quickstart.json stop
```

命令结果显示为文本和表格，无需编写或解析 JSON。需要精确原始字节时给 `read` 加 `--raw`。后台 `output-raw` 的终端输出在 `start` 返回的 stdout 日志中；希望直接看到终端输出时使用 `run --input-file source=./example.log --output-raw screen`。`read` 是当前保留缓冲的快照，Output 订阅才会读到末尾后持续等待。更多纯命令行参数见 [CLI 参考](reference/cli.md)；已有配置文件仍可通过 `--config FILE` 启动。

Core 不保存日志。读取到的记录可能已不是源文件开头；缓冲覆盖不会等待慢 Output。保存需求见 [输出与保存](guides/archive.md)。
