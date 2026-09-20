# 快速开始：采集并读取一条日志

本页完成一个完整流程：启动文件采集，追加一条日志，通过 CLI 读取，再停止实例。先完成 [安装与构建](installation.md)，以下命令在仓库根目录运行。

这里写入的是便于核对的教学文本，不是真实业务日志。示例使用仓库自带的 `project/examples/basic.json`。

## 1. 准备日志文件

```sh
python3 -c "from pathlib import Path; Path('example.log').touch()"
```

这条命令创建文件；文件已存在时不会清空内容。示例从采集启动时的文件末尾开始，因此旧内容不会被读入。

## 2. 启动采集

```sh
./target/release/log-print --state .log-print/quickstart.json start --config project/examples/basic.json
```

成功时返回 JSON，其中 `started` 为 `true`。这份配置声明了：

| 配置内容 | 本例作用 |
| --- | --- |
| `input-file`，插件 ID 为 `file` | 跟随 `example.log` 的新增字节 |
| 流 ID `logs` | 后续读取时使用的名字 |
| `from_start: false` | 从启动时文件末尾开始 |
| `output-raw`，插件 ID 为 `raw` | 把 `logs` 的原始字节输出到插件 stdout |

后台运行时 stdout 由主程序重定向，不一定显示在当前终端。下一步用 `read` 主动查看 Core 中的数据。

## 3. 写入并读取

```sh
python3 -c "open('example.log','ab').write(b'temperature=23.5\n')"
./target/release/log-print --state .log-print/quickstart.json read logs --raw --wait-ms 1000
```

首次执行本流程时，预期输出为：

```text
temperature=23.5
```

`--raw` 只输出日志原始字节；`--wait-ms 1000` 在当前没有记录时最多等待 1 秒。`read` 读取一页数据后退出，不会一直跟随。

需要看到序号、流身份和下一页位置时，去掉 `--raw`：

```sh
./target/release/log-print --state .log-print/quickstart.json read logs
```

同一实例中重复执行 `read` 默认仍从序号 1 请求，可能再次看到已读内容。连续读取的方法见 [读取与管理实例](guides/read.md)。

## 4. 查看状态并停止

```sh
./target/release/log-print --state .log-print/quickstart.json status
./target/release/log-print --state .log-print/quickstart.json stop
```

`status` 用于查看流和插件状态；`stop` 请求停止这一实例并等待状态文件移除。原始 `example.log` 不会因停止采集而被删除。

本例没有开启 Core 保存，也没有创建 `output-file` 归档。内存缓冲有上限，不能把一次成功读取当作长期保存。

## 接下来

- 换成自己的日志路径：[跟随日志文件](guides/file.md)。
- 直接运行并采集程序：[采集程序输出](guides/program.md)。
- 将日志持续保存到文件或 SQLite：[输出与归档](guides/archive.md)。

如果没有读到内容，先检查启动和追加的先后顺序、工作目录，以及命令使用的 `--state` 是否一致，详见 [排查常见问题](troubleshooting.md)。
