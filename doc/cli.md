# CLI 与进程管理

正式入口为 `log-print`。从仓库根目录执行 `cargo build --workspace` 后，二进制位于 `target/debug/`（Windows 带 `.exe`）。下面是接口说明；运行验证记录另见验收报告。

## 最小采集、读取与停止

先创建空的 `example.log`，在项目根目录运行：

```sh
log-print start --config examples/basic.json
log-print status
log-print streams
```

向 `example.log` 追加内容后读取：

```sh
log-print read logs --from 1 --limit 64 --wait-ms 1000
log-print read logs --from 1 --limit 64 --wait-ms 1000 --raw
log-print stop
```

示例默认只保留有界内存数据。`read` 返回一页 JSON，包括记录、范围、epoch 和缺口信息；按返回的下一游标继续读取。`--raw` 仅提取该页记录原始字节，遇到已报告缺口拒绝静默拼接。读取实时数据由 Output 的订阅完成，不需要 CLI 循环轮询。

`--wait-ms` 默认为 0，即一次快照；可设置 0–60000。在首个快照为空时，CLI 使用同一个 Core 连接，每隔最多 20 毫秒重新读取同一游标，在指定的额外等待期限内遇到记录或缺口立即返回。期限到达返回最后一次真实空页，不假称已读到数据；连接和 RPC 错误仍会失败。该参数方便追加文件后的一次读取，持续实时接收仍由 Output 订阅完成。

`run --config FILE` 在前台运行，Ctrl-C 请求关闭本实例。`start --config FILE` 启动后台主进程并等待 Core 就绪，插件原样 stdout 追加到 `<state>.stdout.log`，诊断追加到 `<state>.stderr.log`。主进程启动成功表示 Core 就绪且配置为自动启动的插件已连接，或已报告正常完成；业务效果仍须结合 `status` 和实际数据核验。

所有命令都接受 `--state FILE`，默认 `.log-print/state.json`。不同实例使用不同状态文件；一个实例只创建一个 Core。状态文件含管理令牌，不应分享或提交。现有状态文件不会被覆盖；异常断电留下的状态须先确认原实例已停止，再移除失效文件。不要只根据旧 PID 终止进程。

## 启停、状态与配置

```sh
log-print plugin stop raw
log-print plugin start raw
log-print plugin restart raw
log-print config
log-print config --plugin file
log-print config set file --json '{"poll_ms":20}'
log-print call config.patch --json '{"buffer_records":8192}'
log-print call resume --json '{"stream":"logs"}'
```

`status` 保留 Core 的 `streams`、`plugins`、`config`，增加 `supervisor` 和 `plugin_processes`。后者记录直接子进程 PID、运行状态、最近退出结果；已退出插件不会自动重启。`config` 显示解析后的启动配置及来源，`config --plugin` 同时请求当前运行配置；插件离线或不支持配置查询时明确给出 `runtime_error`。`config set` 只允许插件声明的动态项，默认不回写文件；连接、流身份、保存路径等项修改配置后重启实例。保存失败后手动 `resume` 是否成功由 Core 实际恢复结果决定。

`plugin stop` 先经 Core 发送统一 `shutdown`，再等待退出；超时仅终止本主进程持有的那个子进程句柄，并回报 `forced`、控制失败和未知的未完成数据。`stop` 并行停止插件后关闭 Core stdin，并等待其退出。Core 意外退出时主进程回收本实例插件并以失败退出。程序 Input 自己创建的程序后代由该插件的关闭流程处理，不能把插件 PID 已消失当成后代已全部回收。

## 两类绘图插件的在线 session

TUI/WebUI 分别有插件 ID，各自维护 session。以下要求当前实例由 `examples/dashboard.json` 启动（先停止上面的 basic 实例），该配置提供 `web` 插件与 `sensor` 流；创建的新 session 使用不同于预置项的 ID：

```sh
log-print start --config examples/dashboard.json
log-print session list web
log-print session create web --json '{"id":"extra-temperature","title":"温度","series":[{"name":"t","stream":"sensor","pattern":"temperature=(?P<value>[0-9.]+)"}]}'
log-print session get web extra-temperature
log-print session select web extra-temperature
log-print session set web extra-temperature --revision 1 --json '{"window_secs":120,"refresh_ms":200}'
log-print session export web extra-temperature --path extra-temperature.png --format png
```

WebUI 的 `session select` 返回包含所选session的URL；TUI的同名命令切换对应终端当前显示的session；插件不支持的方法明确报错。修改必须提供查询得到的当前 `revision`，并发旧版本会拒绝；成功后使用新版本继续修改。导出可增加 `--revision N`、`--width 1200 --height 700`、`--overwrite`，默认为不覆盖已有文件。导出路径按调用 CLI 的工作目录解析为绝对路径，然后由目标插件保存。动态 session 修改不会重启采集，也不改变其他 session。

TUI 的终端路径在插件 `config.tty` 指定。插件 stdin 不用于 IPC；TUI 直接使用指定终端，Core 与插件通过本地 TCP 通信。后台启动时须配置可用 tty 或使用 headless 模式；WebUI 地址从插件状态查询。

## 通用调用与路径

```sh
log-print plugin call web session.get --json '{"id":"extra-temperature"}'
log-print call read --json '{"stream":"sensor","from":1,"limit":64}'
```

`plugin call ID METHOD` 通过 Core 的控制路由调用任意插件方法；`call OP` 调用 Core RPC。超时表示结果可能未知，不会自动重复执行控制操作。原始 `subscribe` 是长期事件接口，不适合一次请求退出的 `call`，请使用 SDK/Output。

配置文件自身路径相对启动命令的工作目录。`bin` 为纯名称时先查 `log-print` 所在目录，再查 PATH；包含相对路径时按配置文件目录解析；绝对路径直接使用。插件 `config` 内的文件路径及 Core 默认保存目录相对主进程启动时工作目录，与 config 文件所在目录不同。CLI 的 `--state` 也相对当前工作目录，跨目录操作建议传绝对路径。

Unix 运行状态与内部配置文件使用 0600 权限，目录使用 0700；Windows 使用 `icacls` 限制为当前用户访问，ACL 设置失败则停止启动（该平台实现仍须实际验证）；内部配置保存在新建私有目录，停止或启动失败后删除。后台日志保留供问题复现，可能含用户日志内容；按需要自行归档清理。所有管理地址都限制为 loopback，令牌不写入诊断输出。
