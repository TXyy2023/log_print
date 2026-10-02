# output-tui

当前 log-print/2 的终端 Output，使用 Ratatui/Crossterm。后台插件通过 SDK 只读订阅全部流；交互视图使用 loopback HTTP 连接共享 `log-view` 引擎。退出 attach 不停止采集。

```sh
log-print start --input-file source=./app.log --output-tui term --tui-archive term=./capture
log-print tui term attach
log-print tui term attach --snapshot --width 140 --height 36
# 浏览器/终端使用同一份页面
log-print tui web attach
```

完整使用说明、功能映射、CLI、终端限制和测试入口见 [用户文档](../../../../doc/public/plugins/output-tui.md)。

`src/client.rs` 负责有界 HTTP 读取及配置提交；`src/ui.rs` 负责输入、编辑 revision 和命令面板；`src/render.rs` 使用 Ratatui 渲染表格、曲线与布局。业务状态、持久化与历史代码在 [log-view](../../../crates/log-view/README.md)。`examples/pty_driver.rs` 仅用于跨系统真实 PTY/ConPTY 测试，不是运行依赖。
