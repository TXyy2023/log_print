# 跟随日志文件

适合已经把日志写入普通文件的应用。`input-file` 读取新增字节，不要求每条日志以换行结尾。

## 配置文件输入

将以下完整配置保存为仓库根目录的 `file.json`，把 `path` 改为自己的日志路径。启动前确保文件存在。

```json
{
  "plugins": [{
    "id": "file", "bin": "input-file",
    "streams": [{"id": "logs"}],
    "config": {"path": "example.log", "stream": "logs", "from_start": false, "poll_ms": 50}
  }]
}
```

```sh
./target/release/log-print --state .log-print/file.json start --config file.json
./target/release/log-print --state .log-print/file.json read logs --raw --wait-ms 1000
./target/release/log-print --state .log-print/file.json stop
```

采集运行期间，由原应用继续写入日志。这里只配置输入，也能通过 CLI 读取 Core；不要求额外配置输出插件。

## 从哪里开始读

- `from_start: false`：从本次启动时的文件末尾开始，适合只看新内容。
- `from_start: true`：从文件开头读取，适合连已有内容一起采集。

这是输入文件位置，与 CLI 的 `read --from` 不同：后者是已进入 Core 的记录序号。

## 文件轮转与短暂消失

插件检测文件身份变化和长度回退，发现替换或截断后建立新 segment，从新内容开头读取；路径短暂消失时等待恢复并报告状态。

轮转期间旧文件未读完的尾部、检测间隔内的多次替换，以及部分无法识别的重写，可能造成未知数量的数据缺失。Windows 上替换能否完成还与写日志程序使用的 API 有关。需要严格保留历史时，应同时保留原始日志文件，不能将轮询监控理解为任何轮转方式下都绝对无损。

字段、动态轮询间隔和平台细节见 [input-file 参考](../plugins/input-file.md)。需要保存采集结果时继续阅读 [输出与归档](archive.md)。
