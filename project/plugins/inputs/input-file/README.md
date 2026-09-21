# input-file

0.1.2 将文件持续跟随和静态快速读取统一到本插件；不再使用 input-replay。
每个实例写入 Core 分配的一条流，配置别名和说明放在 `PluginSpec.streams`。

```json
{"id":"file","role":"input","bin":"input-file","streams":[{"id":"file-data","description":"应用日志"}],"config":{"path":"/absolute/path/app.log","mode":"follow","from_start":false,"chunk_bytes":4096,"poll_ms":50}}
```

- `mode: "follow"` 默认从启动打开文件时的末尾开始；`from_start: true` 从头读已有内容。轮询追加内容，检测文件身份替换、长度缩短及最近最多 64 字节变化，检测后从新段的开头读取。路径暂时消失时等待重新出现。
- `mode: "static"` 总是从头尽快读到首次 EOF 后退出，不做节奏回放、来源时间戳解析或 SQLite 读取。`source_eof` 仅表示读完并完成传输侧发布调用，**不表示下游处理完成**。此时流与仍保留的缓冲继续存在。
- `chunk_bytes` 为 1–65536，默认 4096；`poll_ms` 为 1–60000，默认 50。配置在启动时固定。
- 记录保留原始字节，不按行拆分；`source_seq` 在本次实例中按块递增，`key` 包含运行 ID、文件段与段内偏移。来源序号不是 Core 接收序号。

Core 的有限缓冲会覆盖旧记录，Input 不等待 Output 消费。静态快读不保证完整无损导入。轮转时不排空旧文件尾部；轮询之间发生多次替换、或截断后快速恢复成相同尾部内容，可能无法检测或补回。停止时尚未发送的字节也不保证送达。UDP 只报告本地发送，不能据此确认 Core 接收。

主程序使用 `log-print start --config ...` 启动，按插件 ID 手动停止采集。业务配置不再接受旧 `stream` 字段。
