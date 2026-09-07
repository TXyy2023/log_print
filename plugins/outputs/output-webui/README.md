# output-webui

独立 Rust Output：Rust SDK接收数据，Axum 0.8.9本地服务提供页面，Apache ECharts 6.1.0 Canvas绘制数值曲线。WebUI与TUI独立启停、独立会话和刷新。

## 启动

```json
{
  "id":"web","bin":"output-webui","reads":["sensor"],
  "config":{
    "streams":["sensor"],"web_bind":"127.0.0.1:0",
    "sessions":[{"id":"alpha","title":"Device telemetry","series":[{"name":"Temperature","stream":"sensor","pattern":"temp=(?P<value>[-+0-9.eE]+)"}]}]
  }
}
```

放入主管理config的plugins列表，正常启动后 `log-print status` 的web插件report给出实际URL。默认随机loopback端口；仅接受loopback地址。不自动打开浏览器，不提供远程托管或多人认证。

页面、样式、业务JS及固定版ECharts均编译嵌入二进制，无CDN、远程字体或运行时npm/Node依赖。最终用户只需要插件和现代浏览器，断开互联网仍可访问本地视图。发行资产与许可证见 [SOURCES.md](assets/SOURCES.md)。

## Session、CLI与页面

`session select web ID` 返回该会话URL（`/?session=ID`），不改变其他已打开浏览器视图。左侧选择session；按钮修改时间窗、暂停视图和深浅主题，均应用同一revision检查。页面通过SSE接收配置和数据快照；采集不依赖浏览器固定轮询。显示刷新使用session独立计时，默认100ms。容器尺寸变化自动resize，空数据、非法值计数和缺口可见。

```sh
log-print --state /path/state.json session list web
log-print --state /path/state.json session get web alpha
log-print --state /path/state.json session select web alpha
log-print --state /path/state.json session set web alpha --revision 1 --json '{"window_secs":10,"theme":"light"}'
log-print --state /path/state.json session export web alpha --revision 2 --path /tmp/web.png --format png
log-print --state /path/state.json session export web alpha --revision 2 --path /tmp/web.svg --format svg
```

页面PNG/SVG按钮下载当前服务端冻结窗口；CLI导出还写包含revision、采样数据、轴范围、来源epoch/seq的JSON元数据。二者均由Rust Plotters导出，无浏览器时CLI仍可用。图形渲染细节不等同ECharts截图；SVG为独立字体嵌入文件，体积约11MB。全部边界见 [log-plot](../../../crates/log-plot/README.md)。

HTTP只提供本地页面、`/api/sessions`、`/api/sessions/{id}`、`POST .../patch`、`.../events`、`.../image/png|svg`和health；错误返回结构化JSON，revision冲突409。每插件最多16条SSE连接、2个HTTP导出；慢浏览器不阻塞采集，刷新快照不无限排队。CLI导出顺序处理且在入流锁外执行。停止时结束SSE，退出本服务。

## 重复验收

```sh
python3 plugins/outputs/output-webui/tests/verify_ui.py --pty
python3 plugins/outputs/output-webui/tests/verify_ui.py --skip-build --pty --serve
```

第二条保留独立测试实例并持续生成正弦/余弦数值，输出URL供真实浏览器检查。Ctrl-C仅停止该实例，保留报告、四份图像及元数据、终端ANSI和config。脚本通过标准库运行，不依赖Python图形库。非POSIX省略`--pty`。

HTTP/SSE检查、PTY记录、PNG解码、真实浏览器交互和实际可见终端分开记录。库声称跨平台不代表本软件已经在每个平台完成上述验收。

持续负载入口使用预构建 release 二进制、POSIX PTY、独立临时 Chrome profile，并把插件进程树与浏览器进程树分别采样；需要隔离 Python 环境安装 `psutil` 与 `websocket-client`，以及已安装的 Chrome，不下载浏览器：

```sh
cargo build --release -p app-log-print -p log-core -p input-file -p output-tui -p output-webui
python plugins/outputs/output-webui/tests/sustain_ui.py --duration 180 --rate 1000 --chrome /path/to/chrome --artifacts /path/to/fresh-run
```

每20秒轮流修改一个UI的alpha会话，检查其他会话独立；每10秒保存完整行计数、保留/显示点数、实际浏览器Canvas状态及CLI响应。采集器记录两棵进程树的CPU/RSS原始样本。SDK队列容量与实际已测点数分列：接口没有暴露传输队列实时占用，不以推算值冒充队列测量。报告与首尾浏览器截图留存，结束会停止自己的应用实例和Chrome，并移除临时profile。该有界时长不代表最大稳定速率或数小时稳定性。

本次已完成的[180秒实测报告](SUSTAINED.md)及[随仓库交付的精选证据](evidence/sustained/README.md)包含脱敏报告、原始资源样本、首尾截图和源文件哈希，新clone也可核对；浏览器RSS增长限制保持原样记录。
