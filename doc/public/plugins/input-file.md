# input-file：跟随与静态读取

[使用步骤](../guides/file.md) · [公共配置结构](../reference/configuration.md)

每个实例写入 Core 分配的一条流。`streams[0].id` 是配置别名，实际流 ID 由 Core 返回；流说明放在 `streams[0].description`。以下仅为插件 `config`：

```json
{"path":"/absolute/path/source.log","mode":"follow","from_start":false,"chunk_bytes":4096,"poll_ms":50}
```

| 字段 | 默认值与范围 | 行为 |
|---|---|---|
| `path` | 必填 | 必须为存在的普通文件 |
| `mode` | `follow` | `follow` 持续跟随；`static` 从头快速读取 |
| `from_start` | `false` | 仅跟随模式使用；`true` 包含启动前已有内容 |
| `chunk_bytes` | 4096，1–65536 | 每块字节上限；UDP 编码后还受包大小限制 |
| `poll_ms` | 50，1–60000 | 跟随模式的轮询间隔（毫秒） |

配置在主程序启动时读取一次。改文件或重启单个插件不会重读主配置，需重启主程序才生效；不提供动态参数修改。插件接受 `shutdown`、`config.get`；检测到 Core 连接错误时按失败退出。UDP 没有 TCP 的 EOF 通知，Core 进程崩溃由主程序检测并组织清理。

## 文件边界

`follow` 默认在打开文件时确定末尾位置，再向 Core 注册。检测追加后读取新增字节；检测文件身份替换、长度缩短，或已读位置前最多 64 字节的锚点变化后，建立新 segment 并从开头读取。路径消失时报告并等待重现。记录 `key` 包含运行 ID、段号与段内偏移，`source_seq` 按本实例的发布块从 1 递增，`channel` 为空。

轮转时不排空旧文件尾部；轮询间隔内的多次替换、截断后恢复相同锚点内容可能无法检测或补回。文件轮转能否成功还取决于写入程序与操作系统使用的文件 API。停止时尚未发布的字节不保证送达。

## 静态完成含义

`static` 总从头读取，尽快发出原始字节，遇首次 EOF 后报告 `source_eof` 并退出。本模式合并了旧 input-replay 的静态快读用途，不继承节奏、速度倍率、时间解析或 SQLite 回放。

`bytes_sent` / `chunks_sent` 统计完成传输侧发布调用的字节与块；TCP 返回 Core 接受结果，UDP 只确认本地发送。`downstream_complete` 始终为 `false`：源读完不代表 Output 已处理。Core 的流和仍在缓冲中的内容继续存在，但缓冲有界、满后覆盖，Input 不等待 Output，**静态快速读取不承诺全量无损导入**。

旧 `config.stream` 字段已移除。容量和 UDP 限制见[配置参考](../reference/configuration.md)。
