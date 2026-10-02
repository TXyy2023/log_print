# CLI 参考

```text
log-print [--state STATE] COMMAND
```

默认状态路径 `.log-print/state.json`。命令结果以表格或缩进文本显示，布尔值显示 `yes` / `no`；失败向 stderr 报错并返回非零。`read` 显示记录元数据和 payload 文本，控制字符转义，非 UTF-8 字节显示十六进制；`read --raw` 输出精确 payload 字节。

| 命令 | 作用 |
| --- | --- |
| `run [启动参数]` | 前台启动，Ctrl-C 停止自有进程 |
| `start [启动参数]` | 后台启动，等就绪后返回 PID、日志路径和流 ID |
| `status` | Core、插件报告、子进程退出状态 |
| `streams` | 列出流 UUID、说明、归属与缓冲状态 |
| `stream UUID` | 查看一个流对象 |
| `resolve ALIAS` | 将配置别名解析为真实流 UUID |
| `describe UUID DESCRIPTION` | 修改流的显示说明 |
| `read UUID [--limit 64] [--wait-ms 0] [--raw]` | 当前保留缓冲的快照，limit 1..64；空结果可等待 0..60000 ms |
| `config [--plugin ID]` | 读取运行时采用的启动快照 |
| `plugin start ID [--stream UUID]` | 启动预配置插件；可把未连接 Output 关联到真实流 ID |
| `plugin stop ID` | 请求收尾并等待子进程退出，返回 success/forced |
| `plugin restart ID` | 停止后使用同一启动快照重新启动 |
| `plugin call ID METHOD [KEY=VALUE ...]` | 插件控制方法，如 `config.get`、output-file 的 `status.get` |
| `call OP [KEY=VALUE ...]` | 高级 Core 对象/RPC 操作 |
| `stop` | 停止实例并等待状态文件移除 |

## 纯命令行启动

不用编写配置文件即可启动。每个插件有一个 ID，Input 的流别名默认就是该 ID。Output 默认订阅所有声明的 Input；需要选择来源或订阅派生流时用 `--read OUTPUT_ID=ALIAS`。

```sh
log-print start --input-file source=./application.log \
  --set source.from_start=true --describe 'source=应用日志' \
  --output-raw screen --no-autostart screen
log-print streams
log-print read STREAM_UUID --wait-ms 1000
log-print plugin start screen --stream STREAM_UUID
log-print stop
```

| 启动参数（均可重复） | 含义 |
| --- | --- |
| `--input-file ID=PATH` | 跟随文件；`--set ID.mode=static` 改为静态快读 |
| `--input-program ID=COMMAND` | 启动程序；`--list-text ID.args=ARG` 添加一个程序参数 |
| `--input-tmux ID=PANE` | 接入已有 tmux 窗格 |
| `--output-raw ID` | 持续显示来源字节 |
| `--output-transform ID` | 声明同名派生流；用 `--set ID.number=true` 等配置转换 |
| `--output-file ID=PATH` | 将一条流保存到新的 raw 文件；多 Input 时须用 `--read` 选一条 |
| `--output-sqlite ID=PATH` | 将选中的流保存到新的 SQLite 数据库 |
| `--input ID=BINARY` / `--output ID=BINARY` | 声明自定义插件；业务配置使用下面的字段参数 |
| `--core KEY=VALUE` | 设置 Core 参数，如 `transport=udp`、`buffer_records=8192` |
| `--set ID.KEY=VALUE` | 设置插件业务字段，点号表示嵌套，如 `archive.commit.max_delay_ms=50` |
| `--text ID.KEY=VALUE` | 强制字段保持字符串，如 `source.env.FLAG=false` |
| `--list ID.KEY=VALUE` / `--list-text ID.KEY=VALUE` | 重复添加列表元素；后者强制字符串 |
| `--read ID=ALIAS` | 选择 Output 的来源；重复添加多条来源 |
| `--describe ID=TEXT` | 设置发布流的说明 |
| `--no-autostart ID` | 留待后续 `plugin start` 启动 |
| `--plugin-arg ID=ARG` | 添加插件可执行程序自身的参数 |

无参数的 `start` / `run` 启动一个空实例。路径以调用时的工作目录为基准；含相对路径的自定义插件 BINARY 也以该目录解析。

启动参数形成固定快照，单个插件重启沿用它。已有文件方式 `start --config FILE` / `run --config FILE` 继续支持，不能和其他启动设置混用。完整业务字段见[配置参考](configuration.md)和对应插件参考。

启动程序并保留完整参数：

```sh
log-print start --input-program source=python3 \
  --list-text source.args=-u --list-text source.args=app.py \
  --text source.env.FLAG=false
```

转换后保存到文件（先确认目标父目录存在且目标文件不存在）：

```sh
log-print start --input-file source=./application.log \
  --output-transform numbered --set numbered.number=true \
  --output-file archive=./capture.log --read archive=numbered
```

多流文件归档可用 `--output archive=output-file`、`--read archive=source`、`--set archive.mode=create`、`--set archive.file.format=raw` 和 `--text archive.file.paths.source=./capture.log`，为每条来源重复添加读取声明与路径。可以同时加 `--text archive.sqlite.path=./capture.sqlite` 保存到两个目标。业务字段仍由对应插件校验。

## 命名参数与高级操作

`call` 与 `plugin call` 可直接传 `KEY=VALUE`，也可重复使用 `--arg KEY=VALUE`。`true`、`false`、`null` 和数字自动转换，其他内容保持字符串。`--text KEY=VALUE` 强制字符串；`--list KEY=VALUE` 添加有类型的列表元素，`--list-text KEY=VALUE` 添加字符串元素；`--empty-list KEY` 传空列表。点号设置嵌套字段。重复标量或冲突字段报错。

```sh
log-print describe STREAM_UUID '编译日志'
log-print call stream.describe stream=STREAM_UUID --text 'description=编译日志'
log-print plugin call archive status.get
```

以上说明操作不改变流身份或内容。`--json` 已移除；对象使用点号字段、列表使用重复参数。

`read` 每次从当前最早保留记录取快照，不提供持久历史游标，重复执行可能读到相同记录。需要持续消费使用 Output/SDK 订阅。旧 `--from`、`--epoch`、`config set`、`session` 已移除。

状态文件包含管理凭据，不要提交。已有实例占用状态文件时，新启动会失败；不要删除正在运行实例的状态文件绕过检查。配置文件修改只有主程序重启后生效，不提供 reload 或热配置。

停止命令返回实际收尾结果，例如 `success: yes`、`forced: no`。任一插件失败或强制终止时 CLI 返回非零，restart 在停止失败后不再拉起新进程。任意卡死 Unix 插件若最终被 SIGKILL，其内部清理无法执行；此时来源进程清理状态未确认，不表示完整保存或完整树回收。

## WebUI 命令

`--output-webui WEB` 声明全流只读 WebUI。`--webui-archive WEB=DIRECTORY` 创建配套全流 SQLite 归档；`--webui-history WEB=ARCHIVE_PLUGIN` 绑定同实例配置的 SQLite output-file，两者互斥。已有文件不覆盖。

```text
log-print webui WEB url|streams|capabilities
log-print webui WEB page list|get|create|set|clone|delete|select
log-print webui WEB panel add|get|set|remove
log-print webui WEB series add|set|remove
log-print webui WEB history read|search|context|curve
log-print webui WEB query get|cancel
```

使用 `--page`、`--panel`、`--series`、`--query` 指定对象，所有设置使用命名参数。页面/面板/曲线命令修改后端配置，浏览器同步。`--revision` 可防止覆盖并发修改。历史命令返回固定水位的任务 ID，`query get --query UUID --offset 0 --limit 200` 分页；`history context` 支持 `--seq`、`--byte-offset`、`--before`、`--after`。曲线支持 `--regex` 命名捕获组 value 或 `--field` JSON 字段。时间范围是 Unix 纳秒。完整字段通过每层 `--help` 查看，示例见 [WebUI](../plugins/output-webui.md)。

Core `read.range` 使用 stream、epoch、from、end、limit，只读取对应 epoch 内仍保留的内存，并返回实际 oldest/head/end/next/uncovered_before。现有 read 保持原行为。
