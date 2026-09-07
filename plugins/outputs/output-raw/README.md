# output-raw

原样输出一个或多个 Core 流，不添加前缀、换行或协议字段。

所有插件由 `log-print` 注入 `LOG_PRINT_CORE/PLUGIN/TOKEN/CONFIG`，通过 Rust SDK 的本地 TCP IPC 连接 Core；stdout 不承载协议，诊断写 stderr。下列 JSON 放在插件声明的 `config`。流须同时出现在声明的 `streams`/`reads`，保存由 Core 按声明执行。默认读到即推，不等待换行；每次发布保留原 key 等待明确保存恢复，其他错误退出且报告。

所有插件接受 `shutdown`、`config.get`；下表列出的动态字段接受 `config.patch`，只改本次进程、不写配置文件。其余字段重启插件生效。

```json
{"streams":["program.out"],"path":"/tmp/capture.raw","from":1,"append":false,"overwrite":false,"fail_on_gap":true}
```

| 字段 | 默认/范围 |
|---|---|
| streams | 必填非空流数组，声明对应 reads |
| path | 无：stdout；有：原始文件 |
| paths | 可选每流路径映射，与 path 互斥，须覆盖所有流且路径不同 |
| from | 1：从最早请求位置开始，缺历史得到 gap |
| append / overwrite | false / false；默认新建且已有文件时报错 |
| fail_on_gap | true；缺口报告后停止，false 则继续但不补造字节 |

全部重启生效。每条记录 write_all 后 flush，磁盘不可写等错误不吞掉。flush 表示送达目标写入接口，不等于磁盘断电持久保证。主进程可能重定向插件 stdout，验收时推荐 `path`。

多个流写同一目标只按事件到达顺序拼接，不承诺跨流总序或恢复来源边界。需要区分来源时使用 `paths:{"program.out":"/tmp/stdout.raw","program.err":"/tmp/stderr.raw"}`。重启从相同历史位置并 append 会重复输出；本插件没有持久输出游标，不冒称 exactly-once 文件交付。

`from:0` 表示连接订阅时仅接收新记录，`from:1` 请求历史起点；默认 1。`config.get` 返回填齐默认后的有效值、每字段来源及动态字段。
