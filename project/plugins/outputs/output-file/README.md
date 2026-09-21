# output-file 0.1.2

仅订阅并保存日志；终端展示用 output-raw，加工用 output-transform。

示例配置：

```json
{"streams":["source"],"mode":"create","file":{"format":"jsonl","paths":{"source":"capture/source.jsonl"}},"sqlite":{"path":"capture/logs.sqlite"},"fail_on_gap":true}
```

`file`、`sqlite` 至少选一个。`file.format` 为 `raw`（连接 payload 原字节）或 `jsonl`（每行带完整 Record、格式版本 2）。SQLite schema 2 保存完整 payload、来源通道、来源序号、纳秒时间和派生关系；64 位无符号值用十进制 TEXT 无损保存。配置流名可以是 alias 或真实 UUID；由 CLI 绑定的实际 reads 优先。

运行时仅支持 `mode:create`，拒绝覆盖已有归档。先订阅并取得 Core 原子起点，再创建归档；首次只包括当前内存尚保留的记录。Core 不保存日志，归档不请求指定历史起点或旧 epoch 恢复。内部归档库保留对自身文件检查点的校验/恢复测试，插件进程没有自动重连或历史补取功能。

写入先进入有界队列；默认最多 256 条/16MiB（按序列化大小），活动写入也占用预算。提交默认每 64 条/4MiB/100ms。文件确认执行数据和索引 flush+sync，再原子更新检查点；SQLite 使用事务。双目标各自提交，不承诺跨文件与数据库的原子事务。

收到、写入、提交确认是不同状态。手动 shutdown 先停止接受新 SDK 事件，再排空已接受记录，提交所有目标；只有完成后才回复 stopped。磁盘、校验、锁或提交错误报告 failed。`status.get` 展示各目标 written/confirmed 进度。

插件本地观察到 Core 接收序号跳跃时会记录缺口，默认 `fail_on_gap:true` 停止；设为 false 时记录缺口后继续。未观察到缺口不证明全流水线完整，Core 覆盖和 UDP 丢包仍可能发生，插件停止也不代表 Input 已结束。

路径必须预先具备父目录，已有文件不覆盖。每个文件会产生索引、检查点、锁等旁文件。版本 2 不自动修改旧版归档；升级后请使用新目录。
