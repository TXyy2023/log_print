# 配置参考

配置使用 JSON，顶层包含 `core` 和 `plugins`。主程序读取文件后启动 Core 与插件；编辑磁盘文件不会自动修改已运行实例。

## 完整结构示例

```json
{
  "core": {"buffer_bytes": 4194304, "buffer_records": 4096},
  "plugins": [{
    "id": "file", "bin": "input-file", "autostart": true,
    "streams": [{"id": "logs"}],
    "save": {"enabled": true, "directory": ".log-print/file-data"},
    "config": {"path": "example.log", "stream": "logs", "from_start": false}
  }]
}
```

这个示例为 `logs` 开启 Core 保存。它不是 `output-file` 归档配置；两者区别见 [理解流与保存](../concepts.md)。

## 插件声明

| 字段 | 用途与默认值 |
| --- | --- |
| `id` | 本实例中的插件 ID，管理命令使用它 |
| `bin` | 插件可执行文件 |
| `args` | 启动插件的参数数组，默认空；不是被采集程序的参数 |
| `autostart` | 默认 `true`；关闭后可用 `plugin start ID` 启动 |
| `streams` | 发布流声明，默认空；每项包含 `id`、可选 `parents` 和 `save` |
| `reads` | 可读取的流 ID 数组，默认空 |
| `save` | 对该插件发布流的 Core 保存设置 |
| `config` | 交给插件的业务参数，具体见插件参考 |

例如 `input-program` 的源程序参数放在 `config.args`，不要与插件声明顶层的 `args` 混淆。派生流用 `parents` 声明父流。

## Core 参数

| 字段 | 默认 | 范围 |
| --- | --- | --- |
| `buffer_bytes` | 每流 4 MiB | 64 KiB..1 GiB |
| `buffer_records` | 每流 4096 | 1..1000000 |
| `max_payload_bytes` | 65536 | 1..65536 |
| `read_batch_records` | 64 | 1..64，另有批次字节预算 |
| `queue_records` | 64 | 1..1024 |

条数和 payload 字节数同时限制缓冲，不能把这些数值当作进程总内存上限。`buffer_bytes`、`buffer_records`、`read_batch_records` 支持 Core 运行时修改；其他上述字段需要重启。

## Core 保存参数

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `enabled` | `false` | 是否保存发布记录 |
| `directory` | `.log-print/data` | 保存根目录，按流分目录 |
| `file_bytes` | 64 MiB | 单段上限，至少 256 KiB，按页大小取整 |
| `total_bytes` | 每流 1 GiB | 至少 `2 × file_bytes`；含事务日志预留 |

覆盖顺序为 Core 内置默认 → `core.save` → 插件 `save` → 流 `save`。后层只覆盖明确指定的字段。

保存容量不足时阻塞对应保存流，不自动淘汰历史。修复后可使用 Core `resume`，但应先确认状态；它不是归档插件的 `mode: resume`。

## 路径与运行时修改

相对路径通常从启动 `log-print` 的工作目录解析。被采集程序可另设 `config.cwd`，不要将配置文件目录与运行目录混淆。

```sh
./target/release/log-print config --plugin file
./target/release/log-print config set file --json '{"poll_ms":100}'
./target/release/log-print call config.patch --json '{"buffer_records":8192}'
```

这些示例使用默认状态路径，其他实例需加对应 `--state`。运行时覆盖不会写回 JSON；重启后要保留修改，应同步修改配置文件。是否支持动态修改以各插件字段表为准。
