# output-tui

独立 Rust Output，使用 Rust SDK 订阅同一 Core 的一条或多条流，用 Ratatui 0.30.2 / Crossterm 0.29.0 绘制数值曲线；不承载 WebUI。两库采用 MIT 许可证。

## 配置与真实终端

```json
{
  "id":"tui","bin":"output-tui","reads":["sensor"],
  "config":{
    "streams":["sensor"],"from":1,
    "tty":"/dev/ttys001",
    "sessions":[{"id":"alpha","title":"Device telemetry","series":[{"name":"Temperature","stream":"sensor","pattern":"temp=(?P<value>[-+0-9.eE]+)"}]}]
  }
}
```

Unix 在目标终端运行 `tty`，把返回路径填为 `tty`。这是明确指定的显示终端：插件检查它确实是 TTY，在该终端进入替代屏幕绘图，按该文件描述符读取真实尺寸，退出恢复光标/原屏。此模式通过 CLI 控制，不读取另一个终端的键盘。只有一个UI应占用同一终端。

尺寸适配同时覆盖 Ratatui 在固定 viewport 缩放/清屏时的 backend 查询；Linux 后台插件没有控制终端时，仍使用指定的 PTY 尺寸。修复后的 Linux PTY 缩放、改标题和切换会话已有真实进程回归证据。

插件也支持 `--tty PATH`、`--session ID`、`--headless` 参数，覆盖对应启动配置。如果stdout直接连接终端且没有指定tty，启用键盘：Tab切换session、q/Esc退出。SDK走独立TCP通道，ANSI和协议数据不会混写。Windows支持前台console；显式Unix TTY路径不适用，需实机验证。

监督器后台启动时stdout通常是日志文件；未提供真实tty就报错，不把写ANSI到文件算作正常TUI。`headless:true` 明确关闭绘图，仅向stderr每秒报告session/revision/点数，用于无终端CI；它不构成终端视觉验收。

## 在线控制与导出

```sh
log-print --state /path/state.json session list tui
log-print --state /path/state.json session select tui alpha
log-print --state /path/state.json session set tui alpha --revision 1 --json '{"window_secs":10,"title":"温度","theme":"light"}'
log-print --state /path/state.json session export tui alpha --revision 2 --path /tmp/tui.png --format png
log-print --state /path/state.json session export tui alpha --revision 2 --path /tmp/tui.svg --format svg
```

选定session只改变此终端当前视图；CLI patch只影响目标session。窗口缩放重新布局，低于40×12时显示尺寸提示，采集继续。图形刷新（默认100ms）、SDK入流和控制处理独立；缺口绘为断点。

全部配置、数值与内存界限、暂停语义、导出字体/元数据见 [log-plot](../../../crates/log-plot/README.md)。特别注意：需要LF终止完整数值行；数据采集本身不等待换行。

## 验收

```sh
python3 plugins/outputs/output-webui/tests/verify_ui.py --pty
```

脚本建立自己的管理实例和POSIX PTY，收集有界ANSI记录、执行resize、CLI改标题和session选择，再清理自己启动的实例。它与实际可见终端检查分开记录。`--tty "$(tty)" --serve` 可用于人工或桌面可见终端验收；结束该脚本会恢复屏幕并停止自己的实例。
