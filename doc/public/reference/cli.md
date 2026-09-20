# CLI 参考

```text
log-print [--state STATE] COMMAND [OPTIONS]
```

`--state` 是全局选项，默认 `.log-print/state.json`。本页省略可执行文件目录；未安装到 PATH 时使用 `./target/release/log-print`。

## 实例命令

| 命令 | 行为 |
| --- | --- |
| `run --config FILE` | 前台启动，Ctrl-C 请求收尾 |
| `start --config FILE` | 后台启动，返回启动信息和日志路径 |
| `status` | 查询实例、插件和流状态 |
| `streams` | 查询流状态 |
| `stop` | 请求停止，并等待状态文件移除 |
| `--version` / `--help` | 查看版本或帮助 |

一个状态路径对应一个实例。不要删除仍在运行实例的状态文件来绕过启动冲突；文件中含管理令牌，不应提交或公开。

## 读取命令

```text
log-print read STREAM [--from N] [--limit N] [--epoch EPOCH] [--wait-ms MS] [--raw]
```

| 参数 | 默认 / 作用 |
| --- | --- |
| `--from` | `1`，请求记录序号；`0` 从当前 `head + 1` 请求快照 |
| `--limit` | `64`，一页请求条数，范围 1..64 |
| `--epoch` | 可选，校验历史身份 |
| `--wait-ms` | `0`，无记录且无缺口时等待，范围 0..60000 |
| `--raw` | 只输出 payload，拒绝含缺口的页 |

连续等待新记录时应传入明确的下一条序号；不要反复用 `--from 0` 代替保存游标。`read` 不是 `tail -f`。脚本应使用返回的 `epoch` 和 `next` 连续请求，见 [读取与管理实例](../guides/read.md)。

## 配置与插件命令

```text
log-print config
log-print config --plugin ID
log-print config set ID --json JSON
log-print plugin start ID
log-print plugin stop ID
log-print plugin restart ID
log-print plugin call ID METHOD [--json JSON]
```

插件 ID 来自配置的 `id`。通用方法包括 `config.get` 和 `shutdown`；可动态更新的字段通过 `config set` 调用。`output-file` 额外提供完整 `status.get`。

修改启动配置文件后，正在运行的主程序不会自动重载；单纯 `plugin restart` 使用主程序已加载的插件声明。需要加载新的静态配置时，应正常停止并重新启动实例。

## Core RPC

```text
log-print call OP [--json JSON]
```

这是高级入口，例如 `config.patch` 修改 Core 动态设置、`resume` 处理已修复的保存阻塞。参数应与对应操作匹配，不能用任意插件业务字段替代 Core 字段。

## 帮助中出现的其他命令

代码中保留 `session` 命令用于绘图插件接口；TUI、WebUI 不在当前版本默认构建范围，因此本手册不把它作为可用的图形功能教程。

命令成功一般向 stdout 输出 JSON，错误写 stderr 并返回非零状态；`read --raw` 成功时只输出原始字节。完整参数可用 `log-print COMMAND --help` 查看。
