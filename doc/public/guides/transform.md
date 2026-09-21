# 转换日志

`output-transform` 为日志加编号、加时间戳，或按来源序号有限等待重排，再发布到独立派生流。原流可继续被其它 Output 展示或保存。

## 加编号和纳秒时间戳

在仓库根目录创建 `example.log`，将以下配置保存为 `transform.json`：

```json
{
  "plugins":[
    {"id":"file","role":"input","bin":"input-file","streams":[{"id":"source"}],
     "config":{"path":"example.log","mode":"follow","from_start":true}},
    {"id":"transform","role":"output","bin":"output-transform","reads":["source"],
     "streams":[{"id":"derived","parents":["source"]}],
     "config":{"number":true,"timestamp":true}},
    {"id":"display","role":"output","bin":"output-raw","reads":["derived"],
     "config":{"annotate":false}}
  ]
}
```

```sh
./target/release/log-print --state .log-print/transform.json start --config transform.json
python3 -c "open('example.log','ab').write(b'temperature=23.5\n')"
./target/release/log-print --state .log-print/transform.json streams
```

从 streams 中分别取得 source 和 derived 的真实 UUID，然后 `read <UUID>` 检查。派生 payload 类似 `[n=1] [ts_ns=...] temperature=23.5`；原流仍保留 `temperature=23.5` 原字节。编号作用于输入块，块不一定恰好是一行。

## 有界重排

增加 `"reorder":true,"max_records":128,"max_bytes":4194304,"max_delay_ms":100`。它使用 Input 的 source_seq，按来源流及 channel 分别排序。缺号最多等待窗口，到期或缓冲将满时跳过缺号继续，重复号保留第一条。

该策略适合处理可观察的来源乱序；不能补出 UDP 丢包或 Core 已覆盖的数据，也不保证来源时间戳全局有序。多路 stdout/stderr 保持各自来源顺序。

## 停止

```sh
./target/release/log-print --state .log-print/transform.json plugin call transform shutdown
./target/release/log-print --state .log-print/transform.json status
./target/release/log-print --state .log-print/transform.json stop
```

转换停止会排空已接受的数据和待重排记录；停止请求的 `stopping` 回复只表示开始停止，可继续通过 status 确认最终 stopped。下游归档仍需独立确认提交。详细默认值和限制见 [output-transform](../plugins/output-transform.md)。
