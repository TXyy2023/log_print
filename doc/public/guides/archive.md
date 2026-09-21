# 输出与归档

终端展示使用 `output-raw`；保存文件或数据库使用 `output-file`。Core 只保留有界滚动内存，不为归档提供永久历史。

## 创建新归档

仓库提供三份完整配置，输入均使用 `input-file` 静态读取示例文件。

| 目标 | 配置 |
| --- | --- |
| 原始文件 | `project/plugins/outputs/output-file/examples/file.json` |
| SQLite | `project/plugins/outputs/output-file/examples/sqlite.json` |
| 文件和 SQLite | `project/plugins/outputs/output-file/examples/both.json` |

从仓库根目录执行，目标必须是未使用过的新目录：

```sh
mkdir -p capture/both
./target/release/log-print --state capture/both-state.json start --config project/plugins/outputs/output-file/examples/both.json
./target/release/log-print --state capture/both-state.json streams
./target/release/log-print --state capture/both-state.json plugin call archive status.get
```

示例保存到 `capture/both/replay.raw` 和 `capture/both/records.sqlite`，并维护索引、检查点与锁。已存在的文件会导致明确失败；保留旧文件，给新配置使用另一目录。

## 确认提交再停止

先查看 source 的报告确认已到达此次源文件 EOF，再记录其流 UUID、epoch 和 head。检查归档 `common[UUID].next` 是否达到同一 epoch 的 `head + 1`，并确认无缺口或错误。这是对当前快照的核对，不是所有负载下完整交付的保证。

```sh
./target/release/log-print --state capture/both-state.json status
./target/release/log-print --state capture/both-state.json plugin call archive shutdown
python3 -c "from pathlib import Path; assert Path('project/plugins/outputs/output-file/examples/source.log').read_bytes() == Path('capture/both/replay.raw').read_bytes(); print('bytes match')"
./target/release/log-print --state capture/both-state.json stop
```

归档 shutdown 只有排空已接受记录并提交后才回复完成。队列为空、输入 EOF、插件接到停止请求，都不能单独替代这一步。这个例子的字节对比不等于断电持久性认证。

## 保存自己的流

声明 `role:"output"` 的 output-file，配置 reads、目标路径及 `mode:"create"`。JSONL 保留完整 Record；raw 只连接 payload；SQLite 同时保留元数据和二进制。

后启动的 Output 可以通过 `plugin start <插件名> --stream <真实流UUID>` 关联已运行流，流 ID 可从 `streams` 获取。首次订阅只能得到当前内存仍保留的部分，已经覆盖的历史不会自动恢复。本版本不提供 live resume；完整参数见 [output-file](../plugins/output-file.md)。
