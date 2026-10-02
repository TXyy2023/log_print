# 操作流程

以下 `LOG_PRINT_BIN` 为已检查的 CLI 完整路径。示例采用 macOS/Linux shell；PowerShell 使用 `& $LogPrintBin` 调用同一命令。所有路径先替换为任务的真实路径；不要把已有日志文件改写成示例内容。

## 跟随一个现有文件

为任务选择独立状态路径。输入必须是已存在的普通文件，优先使用绝对路径。纯命令行启动无需编写配置文件，Input 的流别名等于插件 ID：

```sh
LOG_PRINT_STATE="/absolute/task-directory/state.json"
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" start \
  --input-file source=/absolute/path/application.log \
  --describe 'source=Application log' \
  --output-raw screen --no-autostart screen
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" status
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" streams
```

文件默认从打开时的末尾开始。用户要求包含已有内容时加 `--set source.from_start=true`；一次静态快读用 `--set source.mode=static`，到首次 EOF 退出，仍不承诺全量无损导入。

结果显示为文本和表格。从 `streams` 的 OWNER 和 DESCRIPTION 选出真实 UUID，保存为 `LOG_PRINT_STREAM`，不要把 `source` 别名传给 `read`；也可先执行 `resolve source` 查询：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" stream "$LOG_PRINT_STREAM"
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" read "$LOG_PRINT_STREAM" --limit 64 --wait-ms 1000
```

`read` 显示记录元数据与 payload，控制字符转义，非 UTF-8 字节显示十六进制。需要精确原始字节时追加 `--raw`。`wait-ms` 只等待非空页或 gap；已有记录时可立即返回同一快照。无新日志可以是源未输出、follow 默认跳过旧内容、源程序尚未 flush 或插件失败，先查看 `status`，不要无限重读。

按需启动预声明的实时显示输出：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" plugin start screen --stream "$LOG_PRINT_STREAM"
```

后台 `start` 将插件 stdout 写入启动结果返回的日志路径。直接在终端持续显示可用 `run --input-file source=PATH --output-raw screen`，Ctrl-C 会收尾自有子进程。

只在实例由本次任务创建或明确要求停止时执行：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" stop
```

确认退出码为 0、顶层 `success: yes`，且状态文件已移除。实例停止结果的 core 与 plugins 各自包含收尾信息，确认没有 `forced: yes` 或失败；顶层没有统一的 forced 字段。保留用户需要的日志/归档，失败时报告实际结果。

## 程序 stdout/stderr 或已有 tmux 窗格

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" start \
  --input-program source=python3 \
  --list-text source.args=-u --list-text source.args=/absolute/path/app.py \
  --set source.shutdown_ms=2000
```

程序不隐式通过 shell 启动，使用管道而非 PTY，stdin 关闭。参数列表优先用 `--list-text`，避免数字或 true/false 被当作有类型的值。stdout/stderr 进入同一流并由 channel 区分；各通道有自己的 source_seq，Core 接收序号不等于跨管道的源端先后。停止采集会终止此模式创建的来源进程组或 Windows Job。环境项使用 `--text source.env.NAME=VALUE`。

接入已存在的 tmux 窗格：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" start --input-tmux source=%3
```

先用 tmux 查询实际 pane ID，不凭示例猜测 `%3`。可选 `--text source.tmux_socket=PATH` 指定独立 socket。此模式只采集接入后的新终端字节，可能含控制码，不能区分 stdout/stderr 或取回历史屏幕。已有 pipe-pane 时拒绝接管；停止只断开自己的管道，不关闭窗格。

## 保存 raw、JSONL 或 SQLite

在主实例启动之前声明 Output。先确认目标父目录存在，选用尚不存在的目标文件。单流 raw 归档可直接使用快捷参数：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" start \
  --input-file source=/absolute/path/application.log \
  --output-file archive=/absolute/task-directory/capture/logs.raw \
  --read archive=source
```

保存到 SQLite 可将 `--output-file` 换为 `--output-sqlite archive=PATH`。同时保存到文件与 SQLite，用 `--output-file archive=PATH --text archive.sqlite.path=DB_PATH`；选择 JSONL 文件格式加 `--set archive.file.format=jsonl`。JSONL 是用户选择的归档格式，CLI 查询结果仍显示为文本。

留待手动启动时加 `--no-autostart archive`，然后使用真实流 UUID 绑定：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" plugin start archive --stream "$LOG_PRINT_STREAM"
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" plugin call archive status.get
```

订阅从当前仍保留的最早记录开始，不恢复已被 Core 覆盖的记录。需要从源头开始捕获时，将来源设为 `--no-autostart source`，确认 Output 就绪后再启动来源；慢 Output 仍可能落后于有限缓冲。

检查 status.get 的错误、缺口、目标 written/confirmed 和 common 中该 UUID 的 next。同一 epoch 下，公共确认游标达到刚观察到的源 head + 1，只证明追至该次快照；队列为空或源 EOF 不能单独证明完成。

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" plugin stop archive
```

停止会排空已接受的数据并提交；尚未进入 SDK 队列的数据不在该承诺内。检查成功结果再根据归属决定是否停止整个实例。已有目标拒绝覆盖，当前不支持 live resume；失败/超时后保留目标及旁文件，先检查状态，不立即重启写同一路径。

## 高级参数与启动快照

`config [--plugin ID]` 只读启动快照；`plugin restart ID` 沿用该快照。已有配置文件仍可用 `--config FILE` 启动，不能与纯命令行启动设置混用。

常用流说明可直接执行 `describe UUID TEXT`。高级操作使用命名参数：

```sh
"$LOG_PRINT_BIN" --state "$LOG_PRINT_STATE" call stream.describe \
  --arg "stream=$LOG_PRINT_STREAM" --text 'description=编译日志'
```

`--arg KEY=VALUE` 中 true/false/null/数字有类型，其余为字符串；`--text` 强制字符串。点号表示嵌套字段；列表使用重复 `--list` 或 `--list-text`，空列表使用 `--empty-list KEY`。旧 `--json` 已移除。没有 config set、config.patch、session、read --from 或 read --epoch。

完整启动参数和业务字段见仓库当前的 [CLI 参考](../../../../doc/public/reference/cli.md)、[配置参考](../../../../doc/public/reference/configuration.md)及插件参考。转换使用独立派生流，原流保持原样；通过 `--output-transform derived --set derived.number=true` 声明，其他 Output 用 `--read ID=derived` 订阅。

## WebUI 与完整上下文

```sh
"$LOG_PRINT_BIN" --state "$STATE" start --input-file source=PATH --output-webui web --webui-archive web=DIRECTORY
"$LOG_PRINT_BIN" --state "$STATE" webui web url
"$LOG_PRINT_BIN" --state "$STATE" webui web history search --stream source --regex ERROR
"$LOG_PRINT_BIN" --state "$STATE" webui web query get --query QUERY_UUID --offset 0 --limit 200
```

Page/panel/series 命名命令控制所有显示设置；归档查询返回固定水位的任务 ID，按 next 翻页。检查 coverage 中的实际归档范围、缺口和错误，无归档时只有 Core 当前内存范围。Page 数据库只保存配置，不充当日志归档。
