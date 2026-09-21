# output-file：保存日志

订阅并保存日志；不会向终端显示日志内容，也不改变 Core 的有界滚动内存行为。操作教程见[输出与归档](../guides/archive.md)。

```json
{
  "streams":["source"], "mode":"create", "fail_on_gap":true,
  "file":{"format":"jsonl","paths":{"source":"capture/source.jsonl"}},
  "sqlite":{"path":"capture/records.sqlite"},
  "commit":{"max_records":64,"max_bytes":4194304,"max_delay_ms":100},
  "queue":{"max_records":256,"max_bytes":16777216}
}
```

`file`、`sqlite` 至少选一个。每个流使用独立文件路径；路径、索引、检查点和锁不能互相别名。父目录须先创建。已有目标拒绝覆盖。CLI 绑定实际 UUID 后，单流配置仍沿用其唯一目标路径。

| 配置 | 默认及范围 |
| --- | --- |
| streams | 流 alias / UUID；实际授权 reads 优先 |
| mode | 运行时仅支持 create；旧版 resume 不适用于当前 Core |
| file.format | raw 或 jsonl |
| fail_on_gap | true；观察到接收序号缺口后保存缺口并失败，false 则记录后继续 |
| commit.max_records | 64；1–65536 |
| commit.max_bytes | 4 MiB；1 字节–64 MiB |
| commit.max_delay_ms | 100；1–60000 ms |
| queue.max_records | 256；1–65536 |
| queue.max_bytes | 16 MiB；1 MiB–1 GiB，按包含元数据的序列化大小计算 |

## 格式与提交

`raw` 连续写 payload，不插入换行、前缀或记录边界；空 payload 仍保留在摘要索引中。`jsonl` 每行是 `{"format_version":2,"record":{...}}`，payload 为字节数组，可无损还原二进制。

SQLite schema 2 保存完整 Record。payload 使用 BLOB，64 位无符号序号和纳秒时间使用十进制 TEXT，派生关系使用 JSON，包含来源 `channel` 和 `source_seq`。Record 已不包含 Core 保存状态。唯一键为 `(stream,epoch,seq)`。

收到、写入和提交确认不同。独立工作线程顺序处理归档，按条数、字节数或等待时间触发提交。文件先同步数据及索引，再原子替换检查点；SQLite 在 WAL + synchronous=FULL 下提交事务。文件和 SQLite 各自提交，双目标没有跨目标原子事务。100 ms 是触发等待上限，不是磁盘确认延迟保证。

队列配额包含活动写入；SDK 另有固定事件队列和每订阅待处理帧，因此队列预算不等于整个进程 RSS 上限。下游处理变慢不会使 Input 等待它完成，Core 仍可覆盖旧记录。

## 状态与停止

`plugin call archive status.get` 返回队列、目标 written/confirmed 游标、缺口及错误。`common[UUID].next` 为所有目标均已确认之后的下一序号；同一 epoch 下达到源 `head + 1` 才表示追至这次快照，不能由队列为空单独推断。

`shutdown` 先关闭 SDK 接收并取消订阅，排空已接受事件和工作队列，提交目标并报告状态，然后回复 `stopped:true`。尚未进入 SDK 队列的数据不属于已接受集合。插件停止不等于 Input 完成或全流水线已排空。磁盘、同步、数据库、校验或连接错误不会返回完整成功。

超时表示结果未知，插件可能仍在收尾。先检查进程与状态，勿立即另启进程写同一目标。

## 历史与版本边界

首次订阅起点由 Core 原子确定为当前仍保留的最早记录；之后持续等待新记录。没有指定历史起点、自动重连或旧 epoch 补取。插件本地观察序号跳跃时记录缺口；没有观察到缺口也不证明全链路完整。

内部归档库保留自身检查点校验和恢复能力及测试，但当前插件不开放 live resume。版本 2 不自动转换旧版归档；升级使用新目录，保留旧目标与旁文件。文件同步的断电效果依赖文件系统与硬件，本版测试不等于真实断电认证。

配置为主进程启动快照，修改须重启主进程。验证入口为 `cargo test -p output-file` 和 `python3 quality/tests/v2/outputs.py`；历史套件已归档，不用于本版验收。
