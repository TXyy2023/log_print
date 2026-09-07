# input-file

跟随普通日志文件的新增字节。

所有插件由 `log-print` 注入 `LOG_PRINT_CORE/PLUGIN/TOKEN/CONFIG`，通过 Rust SDK 的本地 TCP IPC 连接 Core；stdout 不承载协议，诊断写 stderr。下列 JSON 放在插件声明的 `config`。流须同时出现在声明的 `streams`/`reads`，保存由 Core 按声明执行。默认读到即推，不等待换行；每次发布保留原 key 等待明确保存恢复，其他错误退出且报告。

所有插件接受 `shutdown`、`config.get`；下表列出的动态字段接受 `config.patch`，只改本次进程、不写配置文件。其余字段重启插件生效。

```json
{"path":"/tmp/source.log","stream":"file","from_start":false,"chunk_bytes":4096,"poll_ms":50}
```

| 字段 | 默认/范围 | 生效 |
|---|---|---|
| path / stream | path 必填；stream=file | 重启 |
| from_start | false：从启动时末尾；true：从头 | 重启 |
| chunk_bytes | 4096，1..65536 | 重启 |
| poll_ms | 50，1..60000 | 动态 |

轮询只负责检测文件新增，Output 订阅仍为实时推送。使用 [same-file](https://docs.rs/same-file/latest/same_file/) 的文件身份检测替换；文件长度回退检测截断，已读位置前最多 64 字节的锚点检测快速截断后重新增长。每次检测到截断/替换开新 segment，从新内容开头读取；发布 key 包含本次 run、segment 与字节 offset，status 报告边界。

路径短暂消失时等待恢复并报告。轮转时未读完的旧文件尾部、检测间隔内发生多次替换、重写且锚点完全一致，可能无法恢复或检测，缺失量标为未知，不宣称文件监控绝对无损。普通文件在运行中变为设备/FIFO 会拒绝。

Windows 的读取句柄使用 Rust 默认的 `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`，身份查询没有取消删除共享。不过 Python 3.12 的 `Path.replace` / `os.replace` 调用旧 `MoveFileExW`，该接口仍拒绝覆盖已打开的目标，不能据此声称任意日志写入程序的轮转 API 都可用。[Rust 共享说明](https://doc.rust-lang.org/std/os/windows/fs/trait.OpenOptionsExt.html#tymethod.share_mode)、[Python 官方问题说明](https://bugs.python.org/issue46003)。

`examples/io/verify.py` 的 Windows 替换验收使用临时编译的 Rust `std::fs::rename` 程序。现代 Rust 在必要时使用 `FileRenameInfoEx` 的 `REPLACE_IF_EXISTS | POSIX_SEMANTICS`，可在共享删除的旧目标仍打开时进行真正替换；测试持续保持采集运行，检查完整替换字节及 `reason=replaced`、新 segment。Rust 编译器是该测试的依赖，临时源码和程序测试后清理；不通过关闭采集、截断或跳过来代替替换。[Rust 实现](https://github.com/rust-lang/rust/blob/1.98.0/library/std/src/sys/fs/windows.rs)、[Windows 替换语义](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/4217551b-d2c0-42cb-9dc1-69a716cf6d0c)。对应平台通过情况以最新 CI 产物为准。
