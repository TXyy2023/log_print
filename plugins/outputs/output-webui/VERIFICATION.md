# UI 验收记录 — 2026-09-08

本次本地环境为 macOS 26.5 (25F71) / arm64 / Rust 1.92.0；真实浏览器为内置 Computer Use 选择的 Chrome。以下结果不外推为 Windows/Linux、任意终端或硬件实测。

- `cargo test -p log-plot`：7/7 通过。覆盖跨块正则/JSON Pointer、非法与超长行、会话独立/版本冲突、点数限制、gap/重复、暂停数据及暂停期间断连状态、PNG解码/SVG内嵌字体。
- 最终 `cargo fmt -p log-plot -p output-tui -p output-webui`、对应三包 `cargo clippy -- -D warnings` 和 `node --check assets/app.js` 通过。
- `verify_ui.py --pty --serve`：真实主管理进程→Core→文件Input→两个独立UI插件，6组检查通过；CLI control 与SSE均经真实运行接口。
- 最终代码另以 `python3 plugins/outputs/output-webui/tests/verify_ui.py --pty` 完整重跑，6/6通过并自动停止实例；包含Web `session select` 返回独立会话URL的验证。
- Linux后台指定PTY的resize错误已定位为Crossterm查询了错误终端，尺寸backend适配后Linux六组PTY回归通过；Windows detached信号注册Err不触发退出的处理加入后，macOS六组回归再次通过。正式三平台矩阵由根报告记录。
- P4/P5另完成180秒、1000数值行/秒的双UI+独立Chrome持续负载，18万行全部匹配，8次CLI修改隔离通过；资源原始样本、浏览器RSS增长限制与有界性审查见 [SUSTAINED.md](SUSTAINED.md)。
- 同一数值被拆成两次文件写入 `temp=1` 和 `2.5...LF`，两个UI都得到12.5。连续曲线对应后续真实采集的数据。
- CLI只修改Web alpha session，beta及TUI的revision保持不变；陈旧revision被拒绝。
- POSIX PTY收到真实Ratatui ANSI绘图，120×32→80×24→100×28缩放、CLI改标题和session选择均反映在完整重绘中。ANSI diff不会重复输出未变化空格，检查在resize触发的完整帧上核对标题。
- 真实Chrome已观察动态双曲线；不刷新页面时CLI修改标题/主题/时间窗立即可见。页面Pause按钮与原生下拉选择beta实际操作成功。
- 最终700×900窗口显示两条曲线图例，控件、标题、时间刻度可读；桌面宽度也已检查。临时viewport已恢复，临时浏览器页已关闭。
- 两插件PNG与SVG导出含冻结配置/revision、来源epoch/seq、显示点和采样说明。最终PNG已实际打开检查中文标题、图例、网格与双曲线。SVG验证结构及自带字体，未穷尽所有外部SVG查看器。
- HTML引用及JS/CSS/图表库均由本地服务提供，代码无运行时CDN加载；此次没有关闭整台电脑的网络，不将它写成断网操作实测。
- 自建测试实例已停止，状态文件已移除。临时原始产物路径见 `evidence/review.json`。图像是模拟数值经过真实软件链路生成的验收样本，不是硬件数据。

[WebUI 导出样本](evidence/web-final.png) · [TUI 导出样本](evidence/tui-final.png) · [机器可读观察](evidence/review.json)

实际可见 macOS Terminal 尚未验收：内置 Computer Use 安全检查拒绝访问 `com.apple.Terminal`，原因为该应用不允许通过此工具操作。未绕过限制；PTY与真实可见终端的结果保持分开。
