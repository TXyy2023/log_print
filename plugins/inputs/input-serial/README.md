# input-serial

只读串口输入，覆盖操作系统提供的 UART/USB 串口设备；不发送数据 TX。

所有插件由 `log-print` 注入 `LOG_PRINT_CORE/PLUGIN/TOKEN/CONFIG`，通过 Rust SDK 的本地 TCP IPC 连接 Core；stdout 不承载协议，诊断写 stderr。下列 JSON 放在插件声明的 `config`。流须同时出现在声明的 `streams`/`reads`，保存由 Core 按声明执行。默认读到即推，不等待换行；每次发布保留原 key 等待明确保存恢复，其他错误退出且报告。

所有插件接受 `shutdown`、`config.get`；下表列出的动态字段接受 `config.patch`，只改本次进程、不写配置文件。其余字段重启插件生效。

```json
{"port":"/dev/cu.usbserial-DEVICE","stream":"serial","baud":115200,"data_bits":8,"stop_bits":1,"parity":"none","chunk_bytes":4096}
```

独立枚举：`target/debug/input-serial --list`。枚举不打开设备，列表中的 `verified:false` 表示发现设备不等于实际采集通过。

| 字段 | 默认/范围 |
|---|---|
| port / stream | port 必填；stream=serial |
| baud | 115200，50..4000000，实际可用值由驱动决定 |
| data_bits / stop_bits / parity | 8 / 1 / none；5..8 / 1..2 / none,odd,even |
| chunk_bytes | 4096，1..65536 |

以上均重启生效。复用 [tokio-serial](https://docs.rs/tokio-serial/latest/tokio_serial/)（MIT，底层 serialport 4.10.0 为 MPL-2.0），禁用数据和硬件流控，不调用任何写入接口。打开设备时驱动可能改变设备控制线状态，无法承诺物理引脚完全不变化。没有 TX API，不支持硬件协议解析或全部设备宣称。

应用仅保留一个发布块及 SDK 有界队列；不可暂停物理来源在操作系统/设备缓冲耗尽时仍可能丢数据。驱动未提供可靠溢出计数，始终公开 `physical_overflow_detection:unavailable`、未知缺失量，遇到明显发布等待再报告可能缺口，不把计时当真实丢字节证据。断开/错误退出，由用户手动重启。真实硬件验证需单独记录；PTY 模拟只能证明软件串口路径。
