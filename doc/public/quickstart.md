# 快速开始

先按 [安装](installation.md) 构建，在仓库根目录运行。以下创建的是演示输入。

```sh
python3 -c "from pathlib import Path; Path('example.log').touch()"
./target/release/log-print start --config project/examples/basic.json
./target/release/log-print streams
python3 -c "open('example.log','ab').write(b'temperature=23.5\n')"
```

`start` 和 `streams` 返回 Core 分配的 UUID，以及说明、归属、缓冲范围。把真实 UUID 填到下一条命令：

```sh
./target/release/log-print read STREAM_UUID --raw --wait-ms 1000
./target/release/log-print plugin start screen --stream STREAM_UUID
./target/release/log-print plugin stop screen
./target/release/log-print stop
```

后台 `output-raw` 的终端输出在 `start` 返回的 stdout 日志中；希望直接看到终端输出时使用 `run --config ...`。`read` 是当前保留缓冲的快照，Output 订阅才会读到末尾后持续等待。

Core 不保存日志。读取到的记录可能已不是源文件开头；缓冲覆盖不会等待慢 Output。保存需求见 [输出与保存](guides/archive.md)。
