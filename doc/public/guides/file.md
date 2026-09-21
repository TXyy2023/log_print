# 读取日志文件

`input-file` 支持持续跟随和静态快速读取，按原始字节分块，日志不需要以换行结尾。以下完整配置保存为 `file.json`，把 `path` 改为已有普通文件的绝对路径：

```json
{
  "plugins": [{
    "id": "file", "role": "input", "bin": "input-file",
    "streams": [{"id": "logs", "description": "应用日志文件"}],
    "config": {"path": "/absolute/path/app.log", "mode": "follow", "from_start": false}
  }]
}
```

```sh
./target/release/log-print --state .log-print/file.json start --config file.json
./target/release/log-print --state .log-print/file.json streams
```

`streams` 返回 Core 分配的实际 `id`（UUID）。将下方 `STREAM_ID` 换成该值，即可查看当前缓冲和停止：

```sh
./target/release/log-print --state .log-print/file.json read STREAM_ID --raw
./target/release/log-print --state .log-print/file.json plugin stop file
./target/release/log-print --state .log-print/file.json stop
```

`read` 是一次有限快照，每次从当前仍保留的最早记录开始，默认最多 64 条；反复执行可能重复看到同一段，不是持续订阅。持续处理与保存使用 Output 插件，见[输出与归档](archive.md)。

## 选择读取方式

- 持续跟随新日志：`mode: "follow"`、`from_start: false`，从打开文件时的末尾开始。
- 跟随并包含已有内容：`mode: "follow"`、`from_start: true`。
- 尽快读完整个静态文件：`mode: "static"`，总是从头读取，第一次 EOF 后退出。

静态模式的 `source_eof` 表示源读完及传输侧发布调用结束，**不是下游全部保存完成**。Core 缓冲满时会覆盖未读数据，Input 不等待 Output，因此快读不保证全量导入。插件退出后 Core 仍运行，已有流和缓冲保留至手动停止 Core。

跟随模式检测替换和截断后从新段开头读，路径暂时消失则等待。旧文件尾部和轮询之间的变化可能无法补回，不能理解为绝对无损监控。所有配置随主程序启动固定，改文件后需重启主程序。[完整参数与边界](../plugins/input-file.md)。
