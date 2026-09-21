# 静态文件快速导入

0.1.2 将文件回放用途合并到 `input-file`，不再构建独立 `input-replay`。配置：

```json
{"id":"import","role":"input","bin":"input-file",
 "streams":[{"id":"imported","description":"静态文件"}],
 "config":{"path":"source.log","mode":"static"}}
```

静态模式从字节 0 尽快读取，读完后插件退出，Core 流和剩余缓冲继续保留。它不模拟历史时间间隔，不提供速度倍率、时间戳解析或 SQLite 回放。

源文件读取结束、Core 接受、Output 处理完是三个不同阶段。Core 不等待 Output，快速导入可能覆盖下游尚未读取的记录；需要检查各输出的实际结果，不能把成功退出解释成完整批处理。

[文件插件参数](../plugins/input-file.md) · [完整性边界](recovery.md)
