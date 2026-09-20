# input-program：程序输入

先完成任务教程：[使用步骤](../guides/program.md)。本页用于查询参数和行为边界。

通用程序 stdout/stderr 字节采集，支持任何能启动并输出到标准管道的语言运行时。

下列 JSON 是插件声明中的 `config`，不是完整启动配置。发布流须列入 `streams`，读取流须列入 `reads`；公共结构见 [配置参考](../reference/configuration.md)。

插件接受 `shutdown`、`config.get`。标为动态的字段可通过 CLI `config set` 修改，只影响当前进程；其他字段需更新配置并重启实例加载。

## 配置

```json
{"command":"python3","args":["-u","project/examples/io/source.py"],"stdout_stream":"program.out","stderr_stream":"program.err","chunk_bytes":4096,"shutdown_ms":2000}
```

| 字段 | 默认/范围 | 生效 |
|---|---|---|
| command / args | 可执行文件必填，参数数组默认空；不隐式调用 shell | 重启 |
| cwd / env | 继承工作目录/环境，可覆盖 | 重启 |
| stdout_stream / stderr_stream | stdout / stderr，必须不同 | 重启 |
| chunk_bytes | 4096，1..65536 | 重启 |
| shutdown_ms | 2000，10..10000 | 重启 |

## 行为与限制

两条管道分别发布，保持各自字节顺序，不承诺跨管道时序。采集不会等待换行；源语言自身的缓冲仍需由源程序 flush 或类似 Python `-u` 控制。Input 不为源程序提供 stdin。IPC 令牌等环境变量会从源程序环境移除。

复用 [process-wrap](https://docs.rs/process-wrap/10.0.0/process_wrap/) 的 Unix 进程组和 Windows Job Object（挂起创建、入 Job 后恢复）；关闭时仅清理自己创建的组/Job。非零源退出是失败。源主动脱离 Unix 进程组的守护进程不在组回收承诺内；Windows 与 Linux 的实际回收须按平台验证。停机时尚未确认的管道数据会明确提示，未知余量不虚构为零。
