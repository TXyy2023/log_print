# output-file

通过现有 Core SDK 订阅原始流或派生流，归档到独立文件、独立 SQLite，或同时归档到两者。每个流保持消费顺序；多个流仅保留实际到达顺序，不提供全局排序。

Core 的 `saved` 和本插件的归档提交是两件事。需要插件离线后补齐时，请开启对应 Core 流的保存，并确保历史保留时间足够。未保存流可能被 Core 缓冲覆盖；重新订阅会收到 Gap，插件无法补造丢失记录。插件不读取 Core 内部 SQLite。

## 运行示例

从仓库根目录运行。示例使用随插件跟踪的源文件，三个配置均开启 Core 保存，输出到不同目录。首次执行需确保相应 `capture/file`、`capture/sqlite` 或 `capture/both` 下没有以前的归档目标；已有归档请按下文恢复，不要覆盖。

```sh
cargo build --workspace
# 任选一个配置；当前终端以前台方式运行。
target/debug/log-print --state capture/file-state.json run --config plugins/outputs/output-file/examples/file.json
target/debug/log-print --state capture/sqlite-state.json run --config plugins/outputs/output-file/examples/sqlite.json
target/debug/log-print --state capture/both-state.json run --config plugins/outputs/output-file/examples/both.json
```

在另一个终端检查状态与归档：

```sh
target/debug/log-print --state capture/both-state.json status
target/debug/log-print --state capture/both-state.json plugin call archive config.get
target/debug/log-print --state capture/both-state.json plugin call archive status.get
# 等 replay 已结束且 archive 报告共同进度追上源流后，再停止 archive。
target/debug/log-print --state capture/both-state.json plugin call archive shutdown
cmp plugins/outputs/output-file/examples/source.log capture/both/replay.raw
target/debug/log-print --state capture/both-state.json stop
```

Windows 使用 `target/debug/log-print.exe`；字节比对可用 Python `Path(...).read_bytes()`，不依赖 `cmp`。示例文件模式是 `raw`；将 `file.format` 改为 `jsonl` 可使用每行一个完整 Record 的归档。

## 配置

下列对象放入插件声明的 `config`，对应流必须列在该插件的 `reads`。全部业务字段需要重启，`config.get` 返回填齐默认值后的配置和来源；`config.patch` 返回 `restart_required`。

```json
{
  "streams": ["program.out", "program.err"],
  "from": 1,
  "mode": "create",
  "file": {
    "format": "raw",
    "paths": {
      "program.out": "capture/stdout.raw",
      "program.err": "capture/stderr.raw"
    }
  },
  "sqlite": {"path": "capture/records.sqlite"},
  "commit": {"max_records": 64, "max_bytes": 4194304, "max_delay_ms": 100},
  "queue": {"max_records": 256, "max_bytes": 16777216},
  "fail_on_gap": true
}
```

| 字段 | 约束与默认值 |
| --- | --- |
| `streams` | 必填，非空、无重复，必须包含于插件 `reads` |
| `mode` | 必填，`create` 或 `resume`；已有归档绝不盲目追加 |
| `from` | 默认 `1`，仅影响新归档；`0` 在启动时通过 `read` 固定为当前 head+1，并持久保存 epoch 和明确游标 |
| `file` / `sqlite` | 至少启用一个；省略另一个可使用单目标 |
| `file.format` | `raw` 或 `jsonl` |
| `file.paths` | 恰好覆盖所有流，每流独立文件；首版不支持共享路径 |
| `commit.max_records` | 默认 `64`，范围 `1..65536` |
| `commit.max_bytes` | 默认 `4 MiB`，范围 `1..64 MiB`；包含 Record 元数据和字节数组序列化预算 |
| `commit.max_delay_ms` | 默认 `100`，范围 `1..60000`；从本批第一条开始计时 |
| `queue.max_records` | 默认 `256`，范围 `1..65536` |
| `queue.max_bytes` | 默认 `16 MiB`，范围 `1 MiB..1 GiB`；包含元数据，满时背压 |
| `fail_on_gap` | 默认 `true`；缺口持久记录后停止；`false` 继续但保持不完整标记 |

相对路径以插件继承的**进程工作目录**为基准，通常是启动 `log-print` 时所在目录，与配置文件所在目录无关。插件自动创建父目录。目标、checkpoint、记录索引、锁文件、数据库及其 WAL/SHM 路径必须互不冲突；同一目标使用进程间独占锁。

先校验配置并锁定路径，再解析初始 Core 游标、持久初始化归档，最后按明确的 epoch/next 订阅。`from=0` 因此不会在恢复时再次跳到最新位置。初始化部分完成、缺少状态文件、归档身份或配置不匹配时停止；不会删除已有用户文件或自动重建损坏状态。遇到失败请保留所有伴随文件用于诊断。

## 文件和 SQLite 的持久性

`raw` 只连续写 payload，不添加前缀、换行或 Record 边界；空 payload 保留在伴随索引中。`jsonl` 使用带 `format_version` 的完整 Record，payload 为 JSON 字节数组，二进制、空字节和跨块 UTF-8 均无损。两个格式都每流单独文件。

文件目标维护伴随 checkpoint 与 Record 摘要索引，保存归档身份、操作系统文件身份、已确认长度、内容摘要和各流 epoch/next。提交顺序为数据及索引写入、同步、checkpoint 持久替换；恢复先验证身份和已确认前缀，只有验证通过才截去未确认尾部。文件变短、相同长度内容被篡改、替换文件、索引或状态损坏均会停止。摘要索引用于验证领先目标重新交付的完整 Record，即使 `raw` 仅包含 payload，也不忽略元数据冲突。

SQLite 使用独立 `records`、`checkpoints`、`metadata` 和缺口表。Record 所有字段保留，payload 为 BLOB；无符号 seq、时间和游标使用十进制 TEXT，避免 SQLite 有符号整数溢出。`upstream` 和 `upstream_epochs` 使用 JSON。`(stream, epoch, seq)` 唯一；重复记录必须完整相同才跳过，冲突停止。Record 和 checkpoint 同事务提交，使用 WAL 与 `synchronous=FULL`，恢复检查身份、schema 和数据库完整性，并逐条验证 Record 摘要、记录/Gap 覆盖与 checkpoint 一致，发现删除的记录或内容变更时停止。源 Record 的 `durability` 不代表本插件归档是否已提交。

文件同步与 checkpoint 替换使用操作系统持久化接口；Unix 在 rename 后同步父目录；Windows checkpoint 使用 `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`，不额外宣称已同步父目录。断电保证仍取决于文件系统、驱动器及硬件是否遵守同步要求；进程崩溃测试不等于真实断电认证。

双目标分别保存持久进度，共同位置取两个目标的较小 next；恢复从共同位置继续，领先目标校验已存在记录后跳过。任一目标失败就停止，不自动降级。文件与 SQLite 之间没有原子事务，一个目标领先是可观察和可恢复的状态。

## 批次、状态和停止

独立归档线程顺序处理目标，磁盘阻塞不占用异步控制循环。批次达到条数、字节数或从第一条计起的时间限制时提交；低流量也定时提交。大于批次字节预算的合法单条 Record 独立提交。100 ms 是触发提交的默认等待上限，**不是磁盘持久确认延迟或吞吐保证**；慢磁盘会延长实际完成时间并向上游背压。

工作队列的条数和字节配额包括正在处理的 Record，满时不继续从 SDK 取数据。SDK 另有固定 64 事件队列，以及各订阅连接至多一个正在解码/等待入队的帧；总内存还包含有界提交批次、RPC 队列和驱动缓存，不应将工作队列上限解释为整个进程 RSS 上限。

状态通常每 500 ms 汇总一次，包含已接收记录/字节、工作队列占用、每目标每流的持久游标、共同进度、缺口及错误。`received_records` 指此进程从 SDK 取出并纳入排空责任的记录数，包含尚未写入的记录；队列归零不单独证明已同步。文件已写入、SQLite 待提交和已持久确认位置由目标状态区分。report RPC 的延迟或失败不会触发无限报告排队。Core 对 report 限制为 16 KiB；流较多时汇总显式标注 `report_truncated`、`total_streams`，省略部分流明细，保留总体缺口统计。通过 `plugin call archive status.get` 获取全部流和目标的完整状态。

`shutdown` 先关闭 SDK 接收端并取消订阅，排空关闭时已成功进入 SDK 队列的数据、主循环手中的待入队记录和工作队列，提交、同步并保存 checkpoint 后才回复。关闭边界前仍在 socket 或 SDK 待入队帧中的数据不属于已接收集合，后续通过持久游标恢复。插件排空不代表 Core 全部历史或完整流水线已排空。

Core 控制等待为 10 秒；超时表示结果未知，归档线程可能仍在收尾。不要把超时当作已停止或未执行，也不要立即再次启动写同一目标的进程。强制终止可能留下当前块部分写入和一个目标领先的状态；`resume` 会在校验后恢复未确认尾部，并从确认游标重读。

Gap 默认持久记录后失败；允许继续时也永久保留不完整状态。epoch 变化停止。SDK 断连报告完整性未知，不虚构丢失条数，不自动重连。写入或同步错误、数据库失败、控制连接丢失均不会返回完整成功。

恢复时停止旧插件，将配置 `mode` 改为 `resume` 后启动；保持流、输出格式和所有目标路径一致，保留目标与所有伴随状态。`from` 不参与恢复起点。需要重启 Core 时须保留对应保存历史和 epoch；启动全新未保存 Core 会导致 epoch 不匹配并被拒绝。

## 验证

`cargo test -p output-file` 覆盖存储契约；`python3 plugins/outputs/output-file/tests/verify.py` 驱动真实 Core 与插件，覆盖三种目标、恢复和故障场景。测试使用临时目录，不复用示例归档。

仅当 `LOG_PRINT_ARCHIVE_TESTING=1` 时启用测试注入：`LOG_PRINT_ARCHIVE_FAILPOINT` 选择存储提交阶段并强制进程中断，`LOG_PRINT_ARCHIVE_TEST_DELAY_MS` 为每条 Record 注入阻塞延迟。`LOG_PRINT_ARCHIVE_ERRORPOINT` 可选择文件写入/同步、checkpoint 替换或 SQLite 提交的错误返回；这些是注入错误，不能当作真实磁盘耗尽验收。它们用于恢复与控制超时验证，不属于业务配置。
