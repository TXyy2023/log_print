# 读取与管理实例

管理命令通过 `--state` 定位实例。本页以快速开始的 `.log-print/quickstart.json` 为例；使用其他教程时替换成对应路径。

## 查看流与插件

```sh
./target/release/log-print --state .log-print/quickstart.json status
./target/release/log-print --state .log-print/quickstart.json streams
```

`status` 包含插件、Core 和进程状态；`streams` 只返回流信息。进程存在并不等于源程序在产生日志，也不等于归档已完成。

## 读取原文或结构化记录

```sh
./target/release/log-print --state .log-print/quickstart.json read logs --raw --wait-ms 1000
./target/release/log-print --state .log-print/quickstart.json read logs --from 1 --limit 64
```

`--raw` 输出 payload 原始字节，遇到缺口会拒绝输出该页。默认 JSON 结果包含 `records`、`epoch`、`next`、`head` 和 `gap`，适合脚本或 AI Agent 消费。`payload` 是字节数组，不应假定总是 UTF-8 文本。

`read` 每次取一页，默认最多 64 条，实际也受批次字节预算限制；它不是持续订阅命令。

## 连续读取的游标规则

1. 第一次从 `--from 1` 请求。
2. 检查 `gap`；有缺口时记录并处理，不当成完整历史。
3. 保存结果的 `epoch` 和 `next`。
4. 下一次用 `--from` 传入上次的 `next`，并用 `--epoch` 传入上次的身份。
5. 空页时可加 `--wait-ms 1000` 等待；身份不匹配时先确认是否换了历史。

例如，上一页实际返回 `next: 65` 时，下一页使用 `--from 65`。不要按日志行数推算游标，也不要每次都从 1 请求后直接追加到结果文件。

## 管理插件

```sh
./target/release/log-print --state .log-print/quickstart.json config --plugin file
./target/release/log-print --state .log-print/quickstart.json config set file --json '{"poll_ms":100}'
```

运行时修改只影响当前进程，不写回配置文件。静态字段需要修改 JSON 后按所需范围重启。`plugin start/stop/restart` 操作的是配置中已有插件 ID，不能用插件二进制名称替代 ID。

归档的安全停止需要先确认进度，见 [恢复与完整性](recovery.md)；完整命令列表见 [CLI 参考](../reference/cli.md)。
