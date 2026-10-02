# output-tui：终端工作台与历史上下文

`output-tui` 是本地 Rust Output，使用 Ratatui 0.30.2 与 Crossterm 0.29.0 渲染。与 [output-webui](output-webui.md) 共用 `log-view` 配置、流缓存、分行、数值提取、历史查询及 revision 引擎。运行不需要 Node 或浏览器。

## 启动与连接

```sh
# 默认只提供实时缓存 / Core 当前保留范围
log-print start --input-file source=./app.log --output-tui term
log-print tui term attach

# 新实例可创建本次运行的配套全流归档
log-print start --input-file source=./app.log --output-tui term \
  --tui-archive term=./capture

# 或绑定同实例已配置的 SQLite output-file；与 --tui-archive 互斥
log-print start --input-file source=./app.log \
  --output-sqlite archive=./capture/run.sqlite \
  --output-tui term --tui-history term=archive
```

supervisor 启动后台采集及 loopback API；`attach` 启动连接它的终端视图。`q` 或 Ctrl-C 只退出当前视图，采集、其他终端和后台查询继续。停止实例使用 `log-print stop`。终端进程关闭时恢复光标、鼠标捕获和原终端模式。

```sh
# 连接已有 WebUI 的同一个工作台，浏览器与终端同步
log-print tui web attach

# 自动化、管道或无 TTY 场景：输出一帧无 ANSI 文本
log-print tui term attach --snapshot --width 140 --height 36
```

独立启动 WebUI 和 TUI 插件会获得各自的 Page 数据库。需要相同页面时，直接 attach 已有 WebUI。Page 数据库只能由一个后端打开，多视图连接后端，不直接打开 SQLite。

## 页面与 CLI

所有 WebUI 命令均可将 `webui` 替换为 `tui`，包含 `url`、`streams`、`capabilities`、`page`、`panel`、`layout`、`series`、`history` 和 `query`。`plugin call` 仍可访问相同控制方法。

```sh
log-print tui term page create --name monitor --title '运行监控' --theme dark
log-print tui term page select --page monitor
log-print tui term page set --sidebar-open false --view-x 0 --view-y 0
log-print tui term panel add --kind log --title Logs --stream source \
  --left 0 --top 0 --panel-width 640 --panel-height 320 --column text
log-print tui term panel add --kind curve --title Temperature \
  --left 664 --top 0 --panel-width 400 --panel-height 320
log-print tui term series add --panel CURVE_UUID --name Temperature \
  --regex 'temperature=(?P<value>[0-9.]+)' --color '#5aa9fa'
log-print tui term history search --stream source --regex ERROR
log-print tui term query get --query QUERY_UUID --offset 0 --limit 200
```

终端 `:` 打开同一套命名参数命令面板，可输入 `panel set --format hex`、`series add --name Voltage --field voltage` 等，省略的页面/面板使用当前选择。命令按参数解析，不交给 shell 执行。编辑开始时捕获 revision，冲突时提示并保留后端较新状态。

| 操作 | 按键 |
|---|---|
| 页面列表；新增；复制；删除 | `p`，列表内 `n` / `c` / `d` |
| 全部来源与身份；添加日志 | `s`，列表内 `i` / Enter |
| 新增日志 / 曲线；选择面板 | `a` / `c`；Tab / Shift-Tab |
| 面板 / 页面属性；完整命令 | `e` / `E`；`:` |
| 曲线定义与图例选择 | `y`，列表内 `a` / `e` / `d` / Space |
| 文本 / 正则过滤 | `/` / Ctrl-R |
| 暂停 / 跟随；文本与 hex；元数据 | Space / `f`；`t`；`i` |
| 历史 / 实时 / 全范围搜索 | `h` / `l` / Ctrl-F |
| 历史上一页 / 下一页 | `[` / `]` 或 PgUp / PgDn |
| 选择记录；上下文；完整元数据 | ↑↓；Enter；`I` |
| 覆盖范围、缺口、查询状态 | `o` |
| 移动 / 缩放窗口 | `m` / `r` 后方向键，Enter 保存，Esc 取消 |
| 鼠标移动 / 缩放窗口 | 拖标题 / 右下角 |
| 画布平移 / 缩放 / 全部适配 | Alt-方向键；`+` / `-` / `0` |
| 曲线时间缩放 / 图例 | Ctrl-`+` / Ctrl-`-`；`g` |
| 来源栏 / 缩略图 / 位置锁定 | `b` / `z` / `L` |
| 帮助 / 退出当前终端 | `?` / `q` 或 Ctrl-C |

## 显示映射与持久性

Page 名称、主题、顺序、来源绑定、自由布局、过滤、列配置、曲线、历史定位、暂停及当前选择均由后端持久化；默认在实例状态目录下的 `.tui/`。JSON 配置字段与 WebUI 相同：`state_path`、`listen`、`archive_dir`、`history_plugin`。本次流 UUID/epoch 变化后，按 owner/alias 重新绑定来源；不会自动将旧运行归档当成本次历史。

画布坐标保持 WebUI 的像素单位，100% 时按 **8 px/列、16 px/行** 映射。终端支持自由位置、大小、叠放、隐藏、锁定、平移和缩放；旧 12 列 GridStack 配置也能显示。终端不足 40×12 时提示扩大窗口。日志为有界表格；曲线以 Braille 字符绘制，缺口会断线。

终端使用字符网格，字体大小由终端程序控制，无法逐面板设置物理字体大小或按像素绘制线宽；这些设置仍可通过 CLI 编辑、持久化，并在浏览器中生效。行高和列宽按字符网格换算。左右键横向浏览列，精确列宽、排序、图例及所有附加字段可以通过命令面板编辑。长命令支持 Unicode、方向键、Home/End 和安全粘贴。

## 历史、资源与故障

与 WebUI 相同，历史每页默认 200 行，曲线最多 2000 点、保留极值和缺口，最多同时执行两个扫描任务，可取消。每次查询固定归档提交水位和 Core 读取边界，按 stream/epoch/seq 去重；记录保留字节偏移，可定位跨 Record 分行的上下文。没有归档时只报告内存范围。

`o` 显示实际覆盖、未提交、缺口、归档写入故障；后端断连时保留最后一帧并显示 DISCONNECTED，恢复后自动重连。终端显示缓存最多 64 MiB，单个 HTTP 回复最多 4 MiB，最多并发读取四个面板，每 500 ms 刷新。暂停由后端冻结画面，采集继续。

## 验证入口

```sh
python3 quality/run.py
# 单独运行 TUI：先构建二进制和测试专用 PTY 驱动
cargo build --workspace --bins --examples --locked
python3 quality/ci/python-test.py quality/tests/v2/tui.py
```

Linux/macOS 使用真实 PTY，Windows 使用 ConPTY，覆盖键盘、中文、鼠标拖拽、resize、并发 revision、双视图、退出清理、断连、无 TTY、快照和历史。相同的十项后端进程验收分别对 WebUI/TUI 执行。GitHub 三系统矩阵另跑真实 Chromium 的双浏览器、VueFlow/GridStack、AG Grid、ECharts 与布局恢复测试，保存截图和 trace。
