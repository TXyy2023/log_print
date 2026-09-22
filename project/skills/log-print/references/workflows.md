# 0.1.2 操作流程

以下 `LOG_PRINT_BIN` 为已检查的 CLI 完整路径。示例采用 macOS/Linux shell；PowerShell 使用 `& $LogPrintBin` 调用同一命令。所有示例路径先替换为任务的真实路径；不要把已有日志文件改写成示例内容。

## 跟随一个现有文件

为任务创建独立目录，把下面配置写入 `config.json`。输入必须是已存在的普通文件。业务配置内的路径使用绝对路径，避免继承工作目录造成歧义；只有插件 `bin` 中包含的相对路径明确相对于配置文件目录解析。

```json
{
  "core": {"transport": "tcp"},
  "plugins": [
    {
      "id": "source",
      "role": "input",
      "bin": "input-file",
      "streams": [{"id": "logs", "description": "Application log"}],
      "config": {
        "path": "/absolute/path/application.log",
        "mode": "follow",
        "from_start": false
      }
    },
    {
      "id": "screen",
      "role": "output",
      "bin": "output-raw",
      "autostart": false,
      "config": {}
    }
  ]
}
```

`from_start:false` 从打开文件时的末尾开始；用户要求包含已有内容才改为 `true`。`mode:static` 从头快读，到首次 EOF 退出，但仍不承诺全量无损导入。

```sh
LOG_PRINT_STATE="/absolute/task-directory/state.json"
LOG_PRINT_CONFIG="/absolute/task-directory/config.json"
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" start --config "$LOG_PRINT_CONFIG"
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" status
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" streams
```

从结果中按 `owner:source` 与描述选出真实 `id`，保存为 `LOG_PRINT_STREAM`，不要把 `logs` 别名传给 `read`：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" stream "$LOG_PRINT_STREAM"
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" read "$LOG_PRINT_STREAM" --limit 64 --wait-ms 1000
```

需要原始字节时追加 `--raw`。`wait-ms` 只等待非空页或 gap；已有记录时可立即返回同一快照。无新日志可以是源未输出、`follow` 默认跳过旧内容、源程序尚未 flush 或插件失败，先查看 `status`，不要无限重读。

按需启动预配置的实时显示输出：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" plugin start screen --stream "$LOG_PRINT_STREAM"
```

后台 `start` 将插件 stdout 写入启动结果返回的日志路径，因此 `output-raw` 的字节不会自动显示在本次 CLI 调用的 stdout。`run --config FILE` 则在前台运行 supervisor；停止其 Ctrl-C 会收尾自有子进程。

只在实例由本次任务创建或明确要求停止时执行：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" stop
```

确认退出码为 0、顶层 `success:true`，且状态文件已移除。实例停止结果的 `core` 与 `plugins[]` 各自包含收尾信息，确认没有 `forced:true` 或失败；顶层没有统一的 `forced` 字段。保留配置与用户需要的日志/归档，失败时报告实际结果。

## 程序 stdout/stderr 或已有 tmux 窗格

将输入插件的 `bin` 改为 `input-program`，保留单条流声明，并选择一种 `config`：

```json
{"mode":"spawn","command":"python3","args":["-u","/absolute/path/app.py"],"shutdown_ms":2000}
```

程序不隐式通过 shell 启动，使用管道而非 PTY，stdin 关闭。stdout/stderr 进入同一流并由 `channel` 区分；各通道有自己的 `source_seq`，Core 接收序号不等于跨管道的源端先后。停止采集会终止此模式创建的来源进程组或 Windows Job。

接入已存在的 tmux 窗格：

```json
{"mode":"tmux","tmux_target":"%3"}
```

先用 tmux 查询实际 pane ID，不凭示例猜测 `%3`。可选 `tmux_socket` 是 `tmux -S` 使用的 socket 路径。此模式只采集接入后的新终端字节，可能含控制码，不能区分 stdout/stderr 或取回历史屏幕。已有 `pipe-pane` 时拒绝接管；停止只断开自己的管道，不关闭窗格。不要为满足采集而替换用户已有的 pipe。

## 保存 raw、JSONL 或 SQLite

在主实例启动**之前**将下述预配置 Output 加入 plugins；先创建空的目标父目录，选用尚不存在的目标文件：

```json
{
  "id": "archive",
  "role": "output",
  "bin": "output-file",
  "autostart": false,
  "config": {
    "streams": ["logs"],
    "mode": "create",
    "fail_on_gap": true,
    "file": {
      "format": "jsonl",
      "paths": {"logs": "/absolute/task-directory/capture/logs.jsonl"}
    },
    "sqlite": {"path": "/absolute/task-directory/capture/records.sqlite"}
  }
}
```

`file` 和 `sqlite` 至少保留一个；raw 使用 `file.format:raw`。单流配置可在启动 Output 时绑定真实 UUID，沿用唯一目标路径：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" plugin start archive --stream "$LOG_PRINT_STREAM"
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" plugin call archive status.get
```

订阅从当前仍保留的最早记录开始，不恢复已被 Core 覆盖的记录。需要从源头开始捕获时，在配置中安排 Output 启动，并在确认就绪后才触发可控制的源产生数据；即便如此，慢 Output 仍可能落后于有限缓冲。不要把高速静态文件读取宣传成无损导入。

检查 `status.get` 的错误、缺口、目标 written/confirmed 和 `common[UUID].next`。同一 epoch 下，公共确认游标达到刚观察到的源 `head + 1`，只证明追至该次快照；队列为空或源 EOF 不能单独证明完成。JSONL schema 2 保留元数据与 payload 字节数组；raw 只拼接 payload，SQLite 保存 BLOB。

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" plugin stop archive
```

停止会排空已接受的数据并提交；尚未进入 SDK 队列的数据不在该承诺内。检查成功结果再根据归属决定是否停止整个实例。已有目标拒绝覆盖，0.1.2 不支持 live resume；失败/超时后保留目标及索引、检查点等旁文件，先检查状态，不立即重启写同一路径。

## 其他操作与版本资料

`config [--plugin ID]` 只读启动快照；`plugin restart ID` 不重读磁盘配置。没有 `config set`、`config.patch`、`session`、`read --from` 或 `read --epoch`。

输出转换使用 `output-transform` 的独立派生流，不改写原流。仅在用户需要转换或高级 RPC 时查阅对应的版本文档：

- [CLI 命令](https://github.com/TXyy2023/log_print/blob/ver-0.1.2/doc/public/reference/cli.md)
- [完整配置](https://github.com/TXyy2023/log_print/blob/ver-0.1.2/doc/public/reference/configuration.md)
- [output-transform](https://github.com/TXyy2023/log_print/blob/ver-0.1.2/doc/public/plugins/output-transform.md)
- [归档边界与确认](https://github.com/TXyy2023/log_print/blob/ver-0.1.2/doc/public/plugins/output-file.md)
