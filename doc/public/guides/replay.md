# 回放已有日志

`input-replay` 把已有文件的原始字节发布为普通日志流，可以接入同样的读取、转换和归档流程。回放不会重写负载里的时间或文本。

## 使用固定节奏回放

将以下配置保存为仓库根目录的 `replay.json`。源文件使用仓库随附示例。

```json
{
  "plugins": [{
    "id": "replay", "bin": "input-replay",
    "streams": [{"id": "replay"}],
    "config": {
      "path": "project/plugins/outputs/output-file/examples/source.log",
      "stream": "replay", "timestamp": "none", "interval_ms": 1000
    }
  }]
}
```

```sh
./target/release/log-print --state .log-print/replay.json start --config replay.json
./target/release/log-print --state .log-print/replay.json read replay --raw --wait-ms 1000
./target/release/log-print --state .log-print/replay.json status
```

首次读取可能只拿到已经回放的部分。使用 [分页读取](read.md) 继续获取，直到插件报告回放完成且已读到当前 head，再停止实例：

```sh
./target/release/log-print --state .log-print/replay.json stop
```

## 按源时间控制节奏

`timestamp: auto` 尝试解析行首 RFC3339 时间，或 JSON 行中的 `source_ts_ns` / RFC3339 `timestamp`。也可以明确选择 `rfc3339`、`unix_ms`、`unix_ns`。解析失败时使用 `interval_ms`，不虚构源时间。

第一行立即发布，后续可解析时间间隔除以 `speed`；`speed: 2` 表示将等待间隔缩短一半。`max_gap_ms` 限制缩放后的最长等待。时间倒退或间隔截顶会报告状态。

回放完成后插件正常退出，不循环播放。再次启动会产生新的运行标识，不能将再次回放视作已有输出的无重复续传。特别长的行会拆块，详见 [input-replay 参考](../plugins/input-replay.md)。
