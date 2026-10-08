# output-webui

本地 Rust/Axum 后端，离线 Vue 3 前端。浏览器直接连接本插件的 HTTP/SSE 服务；SDK 只运行在 Rust 后端，Core 凭据不会进入网页。默认绑定 `127.0.0.1:0`，就绪后通过 `log-print webui web url` 查看地址。

## 启动

```sh
# 默认只有实时缓存，不声称拥有全量历史
log-print start --input-file source=./app.log --output-webui web

# 目录内为本次运行创建一个新的 SQLite 归档，由 output-file 全流采集
log-print start --input-file source=./app.log --output-webui web --webui-archive web=./capture

# 使用同一实例中已配置的 output-file SQLite 目标
log-print start --input-file source=./app.log --output-sqlite archive=./capture/run.sqlite \
  --output-webui web --webui-history web=archive
```

两种归档参数互斥。已有文件不会覆盖或自动迁移。配套插件名称为 `web-archive`，使用 `read_all=true`、`discover_streams=true`、`fail_on_gap=false`。显式归档的订阅及 fail_on_gap 保持原配置。Output 就绪后才启动自动采集 Input；动态流可能在发现前已经发布，实际订阅起点会保存在流目录中。

JSON 配置中，在 WebUI 的 `config` 内使用 `archive_dir` 或 `history_plugin`。可指定 `listen` 和 `state_path`。`history_path` 与 `runtime_id` 由 supervisor 生成，不应手写跨实例归档关联。

```json
{"plugins":[
  {"id":"source","role":"input","bin":"input-file","streams":[{"id":"source"}],"config":{"path":"./app.log"}},
  {"id":"web","role":"output","bin":"output-webui","read_all":true,"config":{"archive_dir":"./capture"}}
]}
```

## CLI 编排

无需编写 JSON。命令回复中的 UUID 可用于后续命令。命名参数 `--arg FIELD=VALUE` 支持嵌套列状态、图例选择等字段；`plugin call` 使用相同方法名。

```sh
log-print webui web streams
log-print webui web capabilities
log-print webui web page create --name monitor --title '运行监控' --theme dark
log-print webui web page select --page monitor
log-print webui web panel add --page monitor --kind log --title Logs --stream source --left 24 --top 24 --panel-width 760 --panel-height 480
log-print webui web panel set --panel PANEL_UUID --text error --channel stderr --follow false --metadata true
log-print webui web panel add --kind curve --title Temperature --left 808 --top 24 --panel-width 520 --panel-height 340
log-print webui web series add --panel CURVE_UUID --name temperature \
  --stream source --regex 'temperature=(?P<value>[0-9.]+)' --color '#50c8b8' --width 2
log-print webui web series set --panel CURVE_UUID --series SERIES_UUID --field metrics.temperature
log-print webui web panel set --panel CURVE_UUID --zoom-start 20 --zoom-end 80 --legend true
log-print webui web page clone --page monitor --name monitor-copy
log-print webui web page set --page monitor --title '主监控' --revision 8
```

Page 支持 list/get/create/set/clone/delete/select；panel 支持 add/get/set/clone/remove；series 支持 add/set/remove。Page 的 UUID、唯一名称、标题、主题、顺序、布局、来源绑定、过滤器与曲线定义保存在独立的 SQLite 配置数据库（默认在实例状态目录 `.webui/WEB/pages.sqlite3`）。UUID、owner/alias 绑定在当前 Core 中解析；缺失来源显示“等待来源”。选中 Page、暂停、跟随、列状态、图表缩放和图例通过后端统一管理，所有浏览器同步。提交 SQLite 成功后才确认修改，网页自动携带 revision，CLI 可选择携带。

## 自由工作台

新 Page 默认使用 Vue Flow 自由画布，支持像素位置、窗口尺寸、重叠层级、锁定、隐藏、共享平移缩放视口和缩略图。原有 Page 在打开配置库时补全显示字段，保留 GridStack 模式及原网格坐标；切换模式分别保存两套坐标。来源目录、紧凑工具栏和按需打开的属性栏用于日常调试；日志虚拟表格与曲线不自行重造。

```sh
log-print webui web panel clone --panel PANEL_UUID --title '错误监视' --text ERROR
log-print webui web layout set --place LOG_UUID=24,24,760,480 --place CURVE_UUID=808,24,520,340
log-print webui web page set --layout-mode canvas --view-x 24 --view-y 24 --view-zoom 0.8
log-print webui web page set --show-grid true --snap true --show-minimap false --inspector-open false
log-print webui web panel set --panel PANEL_UUID --font-size 13 --row-height 30 --hidden false --locked true
```

`layout.set` 原子提交多个矩形，`--revision` 可防止覆盖别人正在修改的布局。自由画布宽 320–4000 px、高 220–4000 px，缩放 20%–200%；锁定只限制网页手动拖动，CLI 仍可定位。所有已提交显示状态保存在 Rust 后端，两个浏览器的选择、布局、主题、过滤、视口同步。详见[工作台使用手册](../../../../doc/public/plugins/output-webui.md)。

## 固定历史查询

```sh
log-print webui web history read --stream source
log-print webui web history search --stream source --regex '^ERROR'
log-print webui web history context --stream source --epoch EPOCH --seq 123 --byte-offset 20 --before 10 --after 10
log-print webui web history curve --stream source --regex 'temperature=(?P<value>[0-9.]+)' --time-from 1000000000 --time-end 2000000000
log-print webui web history curve --page monitor --panel CURVE_UUID
log-print webui web query get --query QUERY_UUID --offset 0 --limit 200
log-print webui web query cancel --query QUERY_UUID
```

查询命令返回任务 ID。`query get` 提供进度、最多 200 行及 `next`，继续用固定任务 ID 翻页。文本/正则搜索扫描整个可用归档；上下文按分行结果定位，支持跨 Record、UTF-8 分片及独立 stdout/stderr。时间参数使用 Unix 纳秒，曲线时间坐标使用毫秒。历史曲线复用实时的数值提取，最多 2000 个绘制点，按区间保留首尾、极值和缺口；多条曲线共享点数预算。每条结果保留 stream/epoch/seq/字节偏移，序号与纳秒时间通过 HTTP 返回十进制字符串，避免 JavaScript 精度损失。

查询固定归档提交水位和当前 Core epoch/head，按 stream+epoch+seq 合并去重。查询结果暂存在本插件的独立临时文件中，不写入 Page 数据库。最多同时运行两个扫描任务，保留最近 32 个任务；重启清除旧任务及历史定位，恢复页面配置并绑定本次来源。停止归档仍能读取已提交的前缀。无归档时，历史查询只使用当前 Core 内存范围，实时显示则使用 WebUI 的有限缓存。

返回覆盖信息包括：归档身份、实际起点/提交水位、内存范围、未覆盖前缀、归档与内存之间的缺口、尚未提交范围、归档写入状态和错误。只读 Reader 兼容 SQLite schema 2/3，新归档为 schema 3；raw/JSONL/checkpoint 继续使用原格式。Core 不访问数据库。错误 epoch 不可查询，旧 Core 的归档不会自动绑定到新运行。

## HTTP 与资源

- `GET /api/state`：配置、流目录、运行身份；`GET /api/events`：SSE 同步。
- `POST /api/control`：`{"method":"page.create","args":{"name":"monitor","revision":0}}`；与 CLI 共用控制和查询引擎。
- `GET /api/data?query=UUID&offset=0&limit=200`：固定查询结果页。
- 写请求要求 Origin 等于服务 URL，状态/数据接口验证 Host；Core token 留在 Rust 后端。
- 默认每流缓存最多 4096 Records 或 4 MiB，总 Record 缓存 64 MiB（计入元数据），浏览器日志最多 200 行。暂停显示仍继续采集。暂停帧由 Rust 后端共享，后来打开的窗口也读取同一帧；暂停帧另外最多缓存 64 MiB，改变来源、过滤条件、曲线提取或历史位置时重新冻结。配置库不保存这些日志数据。
- CLI 输出遵守当前 TCP/UDP 帧预算，必要时减少页面行数；超出单条或配置预算时明确提示使用 HTTP，不发送超限帧。

## 构建与验收

```sh
npm ci --prefix project/plugins/outputs/output-webui/frontend
npm run build --prefix project/plugins/outputs/output-webui/frontend
cargo build --workspace --locked
cargo run --locked -p log-print-quality --
```

`assets/` 已提交并嵌入二进制；最终用户运行无需 Node、CDN 或互联网。前端源码与 package-lock 提交到 Git。固定 Vue 3.5.43、Element Plus 2.14.7、Vue Flow Core 1.48.2 / NodeResizer 1.5.1 / MiniMap 1.5.4、Element Plus Icons 2.3.2、GridStack 14.0.0 官方 Vue 集成、AG Grid Community/Vue 36.2.0、ECharts 6.1.0；许可证文本见 `frontend/public/THIRD_PARTY_LICENSES.md`（随前端产物一起打包）。详细验收记录见 [VERIFICATION.md](VERIFICATION.md)。

## 与终端同步

使用 `log-print tui web attach` 可直接连接同一个后端，浏览器和终端共享 Page、配置和历史定位。WebUI/TUI 的状态与历史引擎在 `project/crates/log-view`，Core 凭据只留在 Output 后端。终端字体和曲线笔画按字符网格显示。
