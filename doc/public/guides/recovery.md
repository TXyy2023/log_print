# 完整性与 0.1.2 迁移

Core 只有内存缓冲，不再落库或恢复日志。停止或重启 Core 后生成新的流身份，原缓冲不可恢复；Input 离开但 Core 仍运行时保留流和缓冲。

## 保存结果

output-file 保存实际收到的记录，可写 raw、JSONL 或 SQLite。其独立本地检查点和提交状态不代表 Core 能补回缺失历史。正常停止会排空插件已接收队列并检查写入/提交结果；查看 `plugin stop` 的 `success`、`forced` 和插件报告。强制终止或故障不能表示保存成功。

当前实时插件仅接受 `mode:create`，不会从 Core 恢复归档游标。目标已存在时拒绝覆盖。历史归档请保留原文件及其状态，用旧版本工具处理；本版 SQLite schema 2 移除了 Core durability 字段，不能直接把旧库当成新建目标。

## 旧配置迁移

| 0.1.1 | 0.1.2 |
| --- | --- |
| `log-print/1` | `log-print/2`，旧客户端握手明确失败 |
| Core/插件/流 `save` | 移除；需要保存时配置 output-file |
| 一个 Input 声明多个流 | 一个 Input 一条流；程序 stdout/stderr 以 channel 区分 |
| input-replay | input-file `mode:static` |
| 流配置 id 即运行ID | 配置 id 是别名，运行时使用 Core 返回 UUID |
| `config set` / `config.patch` | 修改文件后重启整个实例 |
| `read --from/--epoch`、历史恢复 | 当前保留快照；持续消费使用 Output 订阅 |
| 复杂转换编码/编辑 | 本版明确支持编号、时间戳与有界来源序号重排 |

不承诺无损快速导入、所有 Output 同步完成、可靠 UDP 或自动重连。来源数据、旧版本归档与当前运行状态应分别保留。

停止命令返回实际收尾结果；任一插件失败或强制终止时CLI返回非零，restart在停止失败后不再拉起新进程。任意卡死Unix插件若最终被SIGKILL，其内部清理无法执行；此时来源进程清理状态未确认，不表示完整保存或完整树回收。
