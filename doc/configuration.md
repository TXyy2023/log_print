# 配置和保存参考

配置是 JSON，顶层 core/plugins。未知结构字段和非法范围拒绝。主程序文件示例见 `examples/basic.json`，插件业务参数见各插件 README。

```json
{"core":{"buffer_records":4096,"save":{"directory":".log-print/data","file_bytes":67108864,"total_bytes":1073741824}},"plugins":[{"id":"input","bin":"input-file","streams":[{"id":"logs"}],"save":{"enabled":true},"config":{"path":"example.log","stream":"logs","from_start":false}}]}
```

保存优先级：Core 内置默认 → core.save → plugin.save → stream.save。未列出的字段继承上层。每个 Input 可以统一选择保存，它的多个流可进一步覆盖；处理插件发布的派生流使用同样规则。stream id 决定保存子目录，禁止路径字符。

## Core 参数

| 字段 | 默认 | 范围与生效 |
|---|---:|---|
| buffer_bytes | 每流4 MiB | 64 KiB..1 GiB；运行时 config.patch |
| buffer_records | 每流4096 | 1..1,000,000；运行时 config.patch |
| max_payload_bytes | 65536 | 1..65536；重启 |
| read_batch_records | 64 | 1..64，另有512 KiB批次字节预算；运行时 |
| queue_records | 64 | 每请求连接1..1024；重启；SDK事件队列固定64 |
| save.enabled | false | 重启实例/相应配置后生效，不隐式改写保存区间 |
| save.directory | .log-print/data | 每流在其下创建独立目录；重启 |
| save.file_bytes | 64 MiB | 至少256 KiB；SQLite单段物理页数上限，向下取整到4096字节页 |
| save.total_bytes | 每流1 GiB | 至少2×file_bytes；计入流目录中的实际文件和一个完整分段大小的事务日志预留；重启 |

缓冲同时限制条数和原始 payload 字节，记录元数据另有固定长度上限。最多128插件、256流；每插件最多128读流/发布流，每流最多32父流。配置上限不是经过所有组合规模验证的容量承诺。

```sh
./target/release/log-print call config.patch --json '{"buffer_records":8192}'
./target/release/log-print config --plugin file
./target/release/log-print config set file --json '{"poll_ms":20}'
```

运行时覆盖不写回文件。status 显示 effective_core；插件 config.get 返回默认值、配置值/运行时值或相应配置状态。不支持安全动态修改的字段明确要求重启。CLI启动参数选择 config/state；业务值由配置和运行时 patch 控制，不能假定任意参数都有单独启动 flag。

## 保存与恢复

SQLite 是 Core 内部实现，文件路径为 `<directory>/<stream>/00000000.sqlite` 等。每段保存 immutable Record、key唯一索引、SHA-256、epoch/owner/parents 与该段已提交 head。提交使用 DELETE journal + synchronous EXTRA；事务成功才确认 saved。原理见 [SQLite 同步语义](https://www.sqlite.org/pragma.html#pragma_synchronous)。

`catalog.jsonl` 持久记录分段身份与数量，`.lock` 独占锁保护同一流目录。目录更新先于该新分段数据的成功确认。重开时校验目录、段连续性、身份、SQLite quick_check、行范围与已提交 head；读取时校验内容哈希。检测到缺段或损坏即阻塞对应保存流，不创建假空历史继续沿用旧序号。

大小上限包含事务预留，可能在磁盘还有空间时保守停止。分段轮转只在总量允许时进行，不自动删除历史。文件移动、人工编辑、还原整个存储目录、掉电或硬件虚报 sync 不是已验证的透明恢复能力。完全删除目录和所有身份元数据后无法从不存在的文件证明曾有内容；新建会得到新 epoch，调用方应校验原 epoch。

写失败立即返回 storage_blocked 或 commit_unknown，并保留失败 key/未知状态。其他流继续。修复权限/可用空间/损坏文件后：

```sh
./target/release/log-print call resume --json '{"stream":"logs"}'
```

resume 不关闭保存、不删除数据、不更换 key。提高容量等静态配置需正常停机、改配置、再启动；存储路径与流身份必须匹配。设备/不可暂停来源超出硬件缓冲的损失量可能未知，见 input-serial 的能力边界。
