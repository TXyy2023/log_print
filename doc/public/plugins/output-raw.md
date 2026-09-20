# output-raw：原始字节输出

先完成任务教程：[使用步骤](../guides/archive.md)。本页用于查询参数和行为边界。

原样输出一个或多个 Core 流，不添加前缀、换行或协议字段。

下列 JSON 是插件声明中的 `config`，不是完整启动配置。发布流须列入 `streams`，读取流须列入 `reads`；公共结构见 [配置参考](../reference/configuration.md)。

插件接受 `shutdown`、`config.get`。标为动态的字段可通过 CLI `config set` 修改，只影响当前进程；其他字段需更新配置并重启实例加载。

## 配置

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

## 行为与限制

全部重启生效。每条记录 write_all 后 flush，磁盘不可写等错误不吞掉。flush 表示送达目标写入接口，不等于磁盘断电持久保证。主进程可能重定向插件 stdout，验收时推荐 `path`。

多个流写同一目标只按事件到达顺序拼接，不承诺跨流总序或恢复来源边界。需要区分来源时使用 `paths:{"program.out":"/tmp/stdout.raw","program.err":"/tmp/stderr.raw"}`。重启从相同历史位置并 append 会重复输出；本插件没有持久输出游标，不冒称 exactly-once 文件交付。

`from:0` 表示连接订阅时仅接收新记录，`from:1` 请求历史起点；默认 1。`config.get` 返回填齐默认后的有效值、每字段来源及动态字段。
