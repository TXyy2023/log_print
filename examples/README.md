# 可运行示例

所有命令在项目根目录执行。运行前 `cargo build --release --workspace --locked`。

`basic.json` 跟随当前目录的 example.log，默认不保存；完整最小流程见根README。

## WebUI 与独立 TUI

```sh
./target/release/log-print start --config examples/dashboard.json --state .log-print/dashboard.json
./target/release/log-print --state .log-print/dashboard.json status
./target/release/log-print --state .log-print/dashboard.json session list web
./target/release/log-print --state .log-print/dashboard.json session set web overview --revision 1 --json '{"window_secs":20}'
./target/release/log-print --state .log-print/dashboard.json session export web overview --revision 2 --path .log-print/overview.png
./target/release/log-print --state .log-print/dashboard.json stop
```

status 的 web 插件 report 包含实际 loopback URL。示例程序用 Python 每100ms写一次温度/电压，可从浏览器选择 overview 或独立 temperature session。Windows 将示例配置 command 改为已安装的 `python`。

TUI 默认不自动启动，防止后台实例把 ANSI 写进日志文件。Unix 在要显示的终端运行 `tty`，将路径加入该插件 config.tty，然后重启实例并运行 `plugin start tui`。或将TUI设为autostart，在可见终端用 `log-print run` 前台运行；Windows使用前台console。两个插件的数据/设置独立，关闭任一个不停止另一个。

更多输入与转换组合及完整真实进程验收在 [io/](io/README.md)。
