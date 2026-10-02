# Python SDK 开发指南 {#python-sdk}

**交付状态（2026-10-03 核对）：** 已在 `codex/python-sdk` 分支的 `6be6992` 实现，尚未合入 main，也不包含在 `ver-0.1.3`。本文描述该实现；各语言状态见 [SDK 总览](index.md)。

要求 Python 3.12+，运行时仅用标准库，通过 `log-print/2` 提供 Python 原生异步入口 `init`、`send`、`records`。协议、身份和控制由库内部处理；不假定已有公开包索引发行版。

## 本地安装 {#install}

在单独目录获取已经实现的源码，构建配套 Core，再安装 Python 包：

```sh
git clone --branch codex/python-sdk https://github.com/TXyy2023/log_print.git log-print-python
cd log-print-python
git checkout 6be6992
cargo build --locked --workspace
python3 -m venv .venv
# POSIX: source .venv/bin/activate
# Windows PowerShell: .venv\Scripts\Activate.ps1
python -m pip install ./project/sdks/python
```

将 `log-print-core` 加入 PATH，或把 `LOG_PRINT_CORE_BIN` 设置为其绝对路径，也可向 `init` 传入 `core_binary="/absolute/path/log-print-core"`。Windows 使用 `log-print-core.exe`。有兼容二进制时可跳过构建；Python 包运行时不需要 Rust 编译器。

## 默认收发示例 {#defaults}

```python
import asyncio
import log_print_sdk as log

async def main():
    async with log.init() as app:
        sent = await app.send("hello\n")
        records = app.records()
        received = await anext(records)
        assert received.payload == sent.payload
        print(received.stream, received.seq, received.text())

asyncio.run(main())
```

没有受管环境时，`init()` 启动一个**隔离的本地 Core**，使用独立的输入和输出身份。上下文退出时关闭连接，仅停止自己拥有的 Core。不同默认 init 会话创建不同实例，不共享全局服务。导入包不会连接或启动进程。Core 二进制须预先存在，库不下载它。

不同输入/输出程序需要共享服务时，使用下方 supervisor 受管配置，仍调用相同的 `init()`。也可同时提供 `address=`、`plugin=`、`token=` 连接已有身份，三个参数必须一起给出；不要硬编码凭据。

## 受管插件 {#managed}

```python
# input.py
import asyncio
import log_print_sdk as log

async def main():
    async with log.init() as app:
        while not app.stopping:
            await app.send("temperature=23.5\n")
            await app.sleep(1)

asyncio.run(main())
```

```python
# output.py
import asyncio
import log_print_sdk as log

async def main():
    async with log.init() as app:
        async for record in app.records():
            print(record.stream, record.seq, record.text())

asyncio.run(main())
```

`init()` 读取 supervisor 交接的 LOG_PRINT_*，使用默认可写或可读流，并处理控制与停止。它不拥有受管 Core。凭据缺失或不完整会报错，不会静默启动新 Core。

保存以下配置为 `python.json`，执行 `log-print --state ./python-state.json start --config ./python.json`，用 `log-print --state ./python-state.json stop` 停止：

```json
{
  "core": {"transport": "tcp"},
  "plugins": [
    {"id":"source", "role":"input", "bin":"/absolute/venv/bin/python",
     "args":["/absolute/input.py"], "streams":[{"id":"samples"}]},
    {"id":"sink", "role":"output", "bin":"/absolute/venv/bin/python",
     "args":["/absolute/output.py"], "reads":["samples"]}
  ]
}
```

Windows 使用 `venv\Scripts\python.exe` 的绝对路径，并在 JSON 中转义反斜杠。args 中的脚本也使用绝对路径，包括带空格的路径。配置用于组织独立进程和权限，无需手写底层协议参数；独立本地模式不需要 JSON。

## API 与保证 {#api}

| API | 用途 |
| --- | --- |
| `async with init(...) as app` | 管理会话资源，复用当前 asyncio 循环；可选 `timeout=30`、`queue_size=64`、`config={}`、`validate=callable`、`core_binary=...`、`core_options={...}`。Core 选项仅用于本地模式；校验器返回有效配置映射 |
| `await app.send(str_or_bytes, *, stream=None, key=None, channel=None, source_seq=None, source_ts_ns=None, upstream=None)` | 返回 Core 已接受的完整 Record；字符串编码为 UTF-8，bytes 原样保留，不自动加换行或拆分。默认 key 标识本会话发送，不构成幂等机制 |
| `app.records(stream="alias")` | 异步迭代保留记录和未来记录；省略 stream 时使用配置的 reads，本地模式经独立 Output 身份读自身来源 |
| `app.config`、`app.stream`、`app.stopping` | 启动配置的副本、自有流 UUID、停止状态 |
| `await app.sleep(seconds)` | 支持协作停止的等待 |
| `await app.streams()` | 查询可见流目录 |
| `await app.read_range(stream, epoch=..., start=..., end=..., limit=64)` | 保留 Core 覆盖元数据，将结果中的记录转换为 Record |
| `await app.create_stream(description, parents=[...])` | 创建获准的流 |
| `await app.report(state=...)` | 报告插件业务状态 |

每个 records 迭代器拥有其订阅，重叠订阅会失败。提前结束时 `await records.aclose()`，上下文退出也会清理。不能并发推进同一迭代器。

`Record` 字段为 `stream`、`epoch`、`seq`、`key`、`payload: bytes`、`source_ts_ns`、`observed_ts_ns`、`upstream`、`upstream_epochs`、`source_seq`、`channel`。使用 `.text(encoding="utf-8", errors="strict")` 显式解码。

仅支持 TCP；受管环境指定 UDP 会明确失败。权限由 Core 检查。转换插件是带自有派生流和父流声明的 Output，参见[转换示例](https://github.com/TXyy2023/log_print/blob/6be6992/project/sdks/python/examples/transform.py)。发布成功表示 Core 接受，不表示消费者处理或持久保存；EOF 不等于全局流完成。

队列有界，慢读可能丢失已被 Core 覆盖的历史。当前 Core 不发送所有缺口事件，因此没有 DataGapError 不证明完整。`records()` 启动时快照读取配置；read_all 用户需用 `streams()` 发现新流并显式订阅。不提供自动重连、重试、持久游标、outbox 或恢复。

错误均为 SDKError 子类：ConfigurationError、PermissionDeniedError、PayloadTooLargeError、ProtocolError、ConnectionLostError、OutcomeUnknownError、DataGapError、StoppedError。远端错误保留 `.code`。OutcomeUnknownError 意味着操作可能已经执行，不可盲目重试。取消保持 Python 的 CancelledError，且不会撤销已发送操作。

初始化报告 sdk_ready，仅表示连接和配置就绪。正常退出仅在插件未给出自己的业务报告时发送 stopped 和 business_complete=false；显式业务报告被保留。业务进度与完成仍由插件负责。shutdown 确认停止中，不代表完成；请求停止时异步迭代正常结束，连接丢失则抛异常。更新配置需要重启。

上下文异常退出报告失败类型，不报告可能含秘密的异常文本。阻塞设备或文件操作应使用超时和 `asyncio.to_thread`，避免阻塞事件循环影响停止。

## 开发与验证 {#verify}

参见[测试合同](https://github.com/TXyy2023/log_print/blob/6be6992/project/sdks/python/TESTING.md)和[插件开发指南](https://github.com/TXyy2023/log_print/blob/6be6992/project/sdks/python/PLUGIN_GUIDE.md)。在上述 Python SDK 源码目录执行：

```sh
cargo build --locked --workspace
python quality/tests/python_sdk.py
```

该流程构建 wheel，安装到干净 venv，在源码目录外验证真实 Core 与 Rust 插件互通，不会发布包。
