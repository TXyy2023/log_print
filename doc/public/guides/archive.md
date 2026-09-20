# 输出与归档

先按保存目的选择输出方式：

| 需求 | 选择 |
| --- | --- |
| 简单输出或导出原始字节 | `output-raw` |
| 保存原始文件，并在校验后继续已有归档 | `output-file` 的 `raw` 文件目标 |
| 保留记录字段和二进制负载 | `output-file` 的 JSONL 或 SQLite |
| 同时保存文件和数据库 | `output-file` 双目标 |

`output-raw` 的 flush 不等于磁盘持久提交，它没有持久消费游标。需要恢复能力时使用 `output-file`。

## 运行文件与 SQLite 双目标示例

示例从随仓库提供的日志文件回放，将结果保存到 `capture/both/`。它已为源流开启 Core 保存，便于归档落后时补读。

首次运行前，确认 `capture/both/` 没有此前的归档；有旧数据时使用 [恢复流程](recovery.md)，不要删除或覆盖旧目标来绕过报错。

```sh
./target/release/log-print --state capture/both-state.json start --config project/plugins/outputs/output-file/examples/both.json
```

此配置产生原始文件 `capture/both/replay.raw` 和 SQLite 数据库 `capture/both/records.sqlite`，并维护恢复所需的伴随状态。

也可以选择另外两个完整示例：

| 模式 | 配置文件 | 建议状态路径 |
| --- | --- | --- |
| 仅文件 | `project/plugins/outputs/output-file/examples/file.json` | `capture/file-state.json` |
| 仅 SQLite | `project/plugins/outputs/output-file/examples/sqlite.json` | `capture/sqlite-state.json` |
| 双目标 | `project/plugins/outputs/output-file/examples/both.json` | `capture/both-state.json` |

## 确认归档进度

```sh
./target/release/log-print --state capture/both-state.json status
./target/release/log-print --state capture/both-state.json plugin call archive status.get
```

等待 replay 报告完成。对 `replay` 流检查归档状态的 `common` 游标：相同 epoch 下，`next` 应达到源流最终 `head + 1`，并且没有缺口或错误。`next` 表示下一条待处理记录，不能直接与 `head` 相等就判断追平。

队列为空或接收条数增加，都不能单独证明归档已持久提交。

## 停止并核对原始字节

确认上一步后：

```sh
./target/release/log-print --state capture/both-state.json plugin call archive shutdown
python3 -c "from pathlib import Path; assert Path('project/plugins/outputs/output-file/examples/source.log').read_bytes() == Path('capture/both/replay.raw').read_bytes(); print('bytes match')"
./target/release/log-print --state capture/both-state.json stop
```

`bytes match` 只核对本例原始文件与源文件的字节一致；不应扩展为所有故障下的持久性证明。

## 换成自己的流

在自己的完整配置中加入 `output-file` 插件，`reads` 和 `config.streams` 指向已有流，并为每个流指定独立输出文件。选择 `file.format: jsonl` 可保留完整 Record；payload 是 JSON 字节数组，不是直接嵌入的日志字符串。

至少启用 `file`、`sqlite` 之一。新归档用 `mode: create`，已有归档用 `mode: resume`。字段和完整 config 示例见 [output-file 参考](../plugins/output-file.md)。
