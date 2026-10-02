<span id="output-webui-workbench-and-history"></span>

# output-webui：自由工作台与历史上下文

本地 Rust/Axum 后端通过当前 SDK 读取 Core。前端使用 Vue 3、Element Plus、Vue Flow 自由画布、GridStack 官方 Vue 集成、AG Grid Community 和 ECharts，构建产物嵌入 Rust 二进制，运行无需 Node、CDN 或互联网。

```sh
log-print start --input-file source=./app.log --output-webui web
log-print webui web url
```

默认实时模式不承诺全量历史。需要完整上下文时，在启动命令加 `--webui-archive web=./capture` 创建本次全流 SQLite 归档，或用 `--output-sqlite archive=./capture/run.sqlite --webui-history web=archive` 绑定同实例显式归档。两种方式互斥，不覆盖已有文件。

![工作台：日志与曲线自由布局](../../assets/webui/workbench.png)

截图使用合成串口文本与温度/电压数据；窗口均由 CLI 编排。

<span id="workbench-controls"></span>

## 工作台操作

左栏列出 Page、面板和所有来源；点击来源即可创建绑定该流的日志窗口。画布只显示配置中的日志和曲线，面板标题用于移动，选中后通过边角调整尺寸。右侧属性面板集中设置位置、尺寸、来源、通道、字体和行高，点击「应用」后提交。过滤、暂停、跟随、历史定位等常用操作保留在窗口内；归档覆盖详情从窗口底部展开。

新 Page 默认使用自由画布，可以重叠窗口、置顶/置底、隐藏/显示和锁定位置。快捷键 `V` 选择、`H` 平移、`0` 适应全部、`+`/`-` 缩放；滚轮平移画布，触控板捏合缩放。侧栏、属性栏、缩略图和点阵均可关闭。表格内滚动和曲线内缩放由对应成熟组件处理。

已有 Page 自动保留原来的 12 列 GridStack 布局；可以从工作台菜单切换「自由画布 / 网格布局」。两套几何位置分别保存，切换不会把旧网格坐标覆盖成像素坐标。旧配置的过滤器、来源绑定和曲线定义保留。

<span id="pages-and-cli-control"></span>

## Page 与 CLI

AI 或脚本可仅通过 CLI 编排界面，无需操作浏览器或手写 JSON。`page get` / `panel get` 查询当前配置，命令返回的 UUID 用于后续操作，`capabilities` 返回布局能力和限制。

```sh
log-print webui web page create --name monitor --title '设备诊断' --layout-mode canvas
log-print webui web page select --page monitor
log-print webui web panel add --kind log --title '串口日志' --stream source \
  --left 24 --top 24 --panel-width 760 --panel-height 480
log-print webui web panel add --kind curve --title '温度趋势' --stream source \
  --left 808 --top 24 --panel-width 520 --panel-height 340
log-print webui web series add --panel CURVE_UUID --name temperature \
  --regex 'temperature=(?P<value>[0-9.]+)' --color '#4777c4' --width 2
log-print webui web panel set --panel LOG_UUID --font-size 13 --row-height 30 --follow true
log-print webui web panel clone --panel LOG_UUID --title '错误监视' --text ERROR
log-print webui web panel set --panel LOG_UUID --hidden false --locked false --z-index 3
log-print webui web layout set --page monitor \
  --place LOG_UUID=24,24,760,480 --place CURVE_UUID=808,24,520,340
log-print webui web page set --page monitor --view-x 24 --view-y 24 --view-zoom 0.8 \
  --show-grid true --snap true --show-minimap false --sidebar-open true --inspector-open false
```

| 显示状态 | 命名参数与范围 |
| --- | --- |
| 布局模式 | `--layout-mode canvas\|grid` |
| 自由画布几何 | `--left` / `--top` 支持负数及小数，范围 ±1,000,000；`--panel-width` 320–4000；`--panel-height` 220–4000，单位 px |
| 网格几何 | 原 `--x` / `--y` / `--w` / `--h`，12 列网格 |
| 原子布局 | 重复 `--place UUID=LEFT,TOP,WIDTH,HEIGHT`；任何一项非法时整批不修改 |
| 层级与可见性 | `--z-index` 0–1,000,000；`--hidden true\|false`；`--locked true\|false` |
| 排版 | `--font-size` 10–24；`--row-height` 22–56 |
| 画布视口 | `--view-x` / `--view-y` ±1,000,000；`--view-zoom` 0.2–2；`--tool select\|pan` |
| 页面显示 | `--theme light\|dark`、`--show-grid`、`--snap`、`--show-minimap`、`--sidebar-open`、`--inspector-open`；布尔参数显式给 true 或 false |
| 当前选择 | `--active-panel UUID` 或 `--clear-active-panel` |

页面配置、布局、视口、显示设置、过滤规则、曲线定义和当前页面独立存入 SQLite。Vue Flow/GridStack 拖拽与缩放、AG Grid 列设置、ECharts 缩放与图例均提交 Rust 后端，CLI 与所有浏览器通过同一引擎同步；视口也共享。网页编辑携带起始 revision，过期提交被拒绝并恢复已提交状态；CLI 可用 `--revision` 做同样检查。手动锁定限制网页拖动，CLI 仍可精确更新位置。

来源绑定保存 owner/alias，在本次 Core 解析 UUID；缺失来源显示“等待来源”。冷启动恢复配置，重新绑定当前运行，不读取上次运行的日志。支持日志多流、通道、文本/正则过滤和文本/十六进制显示；曲线支持多条定义、正则命名捕获组 value 或 JSON 字段路径、时间范围、坐标范围、颜色及线宽。

<span id="historical-context-and-coverage"></span>

## 全量上下文与覆盖边界

```sh
log-print webui web history search --stream source --regex ERROR
log-print webui web history context --stream source --epoch EPOCH --seq 123 --byte-offset 20 --before 10 --after 10
log-print webui web history curve --stream source --field metrics.temperature
log-print webui web query get --query QUERY_UUID --offset 0 --limit 200
log-print webui web query cancel --query QUERY_UUID
```

历史命令返回任务 ID。查询固定 SQLite 提交水位和 Core epoch/head，合并、去重后分页，每条结果保留 Record 身份和字节偏移；实时和历史共用跨 Record 分行与数值提取逻辑。上下文支持独立 stdout/stderr。最多两个后台扫描同时运行，日志页最多 200 行，历史曲线最多 2000 个绘制点。CLI 还受 TCP/UDP 帧预算限制。

界面和 CLI 明确返回归档起点/提交水位、内存范围、未覆盖前缀、缺口、尚未提交和归档故障。停止归档后仍能读取已提交前缀。没有归档时，历史查询只使用 Core 当前内存。新 SQLite 归档为 schema 3，只读兼容 schema 2；原 raw/JSONL 格式不变，已有档案不自动迁移。Core 不访问数据库。

默认每流缓存 4096 Records 或 4 MiB，总记录缓存 64 MiB。浏览器只保留有限结果页；暂停显示仍继续采集。查询临时文件与 Page 数据库分开，重启清除临时定位。

完整命令见 [CLI 参考](../reference/cli.md)，JSON 字段见 [配置参考](../reference/configuration.md)，归档原有设置见 [output-file](output-file.md)。

<span id="share-a-workbench-with-a-terminal"></span>

## 与终端同步

使用 `log-print tui web attach` 可直接连接同一个后端，浏览器和终端共享 Page、配置和历史定位。WebUI/TUI 的状态与历史引擎在 `project/crates/log-view`，Core 凭据只留在 Output 后端。终端字体和曲线笔画按字符网格显示。
