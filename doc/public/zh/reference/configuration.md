<span id="configuration-reference"></span>

# 配置参考

可以使用 [CLI 启动参数](cli.md)直接声明来源、输出和字段，无需配置文件。以下 JSON 文件格式作为可选的批量配置方式，通过 `--config FILE` 加载。

app-log-print 启动时解析命令行设置或读取配置文件一次，保存本运行快照并交接 Core 和插件。后启动或重启子插件使用同一份；只有整个实例停止再启动才采用新设置。未知字段直接报错。

```json
{
  "core": {"transport":"tcp","buffer_records":4096,"buffer_bytes":4194304},
  "plugins": [
    {"id":"source","role":"input","bin":"input-file",
     "streams":[{"id":"logs","description":"应用日志"}],
     "config":{"path":"example.log","mode":"follow"}},
    {"id":"screen","role":"output","bin":"output-raw","autostart":false,
     "reads":["logs"],"config":{}}
  ]
}
```

## Core

| 字段 | 默认值与约束 |
| --- | --- |
| `transport` | `tcp`，可选 `udp` |
| `buffer_records` | 4096；每流 1..1000000 条 |
| `buffer_bytes` | 4194304；每流 1..1073741824 字节，包含序列化元数据成本 |
| `max_payload_bytes` | 65536；1..65536 |
| `read_batch_records` | 64；1..64 |
| `queue_records` | 64；1..1024 |

单条记录超过整个流的字节预算会失败。TCP JSONL 编码帧最大 1 MiB；UDP 编码报文最大 60 KiB（JSON 字节数组有膨胀），正常插件默认 4096 字节块。UDP 成功只表示本地发送；不保证每条到达。

<span id="plugin-declaration"></span>

## 插件声明

| 字段 | 含义 |
| --- | --- |
| `id` | 当前实例唯一插件名，最长 100 字节，保留内部 `__` 前缀 |
| `role` | 必填，只有 `input` / `output` |
| `bin`、`args` | 可执行文件及参数；相对含路径的 bin 以配置文件目录解析，裸名称优先同级已构建工具 |
| `autostart` | 默认 true；false 时用 `plugin start` 后启动 |
| `streams` | 最多一条发布流声明；id 是配置别名，description 最长 4096 字节，parents 表示派生来源 |
| `reads` | Output 可读流别名或真实 UUID；Input 不订阅 |
| `config` | 该插件的静态业务配置 |

一个 Input 只有一条 Core 分配的流；没有声明时在注册时分配。Output 转换时可以拥有另一条派生流。Core 校验单写入者、禁止自订阅和配置反馈环。128 个预配置插件、每插件最多 128 个预配置订阅是资源预算，不是日志分发轮流抢占规则。

`save`、旧存储配置、动态 `config.patch` 均不再有效。`input-replay` 改用 input-file 的 `mode:static`。详情见 [迁移边界](../guides/recovery.md) 及对应插件参考。

<span id="all-stream-outputs-and-webui"></span>

## 全流只读 Output 与 WebUI

插件字段 `read_all` 默认 false，仅允许没有自有流、没有显式 reads 的 Output 使用；该消费者不能创建或发布流。WebUI 与其配套归档启用此权限，其他插件配置保持原语义。

WebUI `config.archive_dir` 对应 `--webui-archive`，`config.history_plugin` 对应 `--webui-history`，二者互斥。`config.state_path` 指定独立 Page SQLite 配置数据库；`config.listen` 默认 `127.0.0.1:0`，只允许 loopback。supervisor 为每次运行生成归档路径与 runtime_id；冷启动只恢复配置，重新绑定本次流。

<span id="terminal-display"></span>

## 终端显示配置

`output-tui` 与 `output-webui` 使用相同的 Output 配置字段与 `read_all` 约束。将 `bin` 设为 `output-tui` 即可；`config` 可设置 `archive_dir` 或 `history_plugin`、`state_path` 与 loopback `listen`。默认 Page 路径位于实例状态目录的 `.tui/`，配置数据库不承担日志归档。见 [终端工作台](../plugins/output-tui.md)。
