# output-webui 工作台验收 — 2026-10-02

本版使用 log-print/2、Rust/Axum 后端和内嵌 Vue 前端。此次执行环境为 macOS 26.5 arm64、Rust 1.92.0、Python 3.14.1；浏览器为 Codex 内置浏览器的独立页面。持续输入为明确标注的合成串口文本、temperature/voltage 与事件数据，不代表真实硬件采集。

## 自动检查

入口：`python3 quality/run.py --report-dir quality/artifacts/validation/output-webui-workbench`。该目录的 `results.json` 记录 12 步全部通过，`complete_selected_suite=true`。

- 前端锁文件安装、Vue/TypeScript 类型检查、许可证汇总和离线构建通过；npm 审计 0 漏洞。
- workspace 格式检查、Clippy 全目标零警告、79 个 Rust 测试和全部二进制构建通过。
- 真实进程场景 63 项：协议 13、supervisor 8、启动故障 4、CLI 5、输入 11、输出 12、WebUI 10；本机没有跳过项目。
- 界面验收期间的来源绑定、深色配色、曲线刻度及窄窗口修正，另外重新执行前端类型/构建与 Rust 二进制构建，并在真实浏览器验证。
- 公开和本地文档均构建并验证链接；公开站点保留隔离检查。

新增 Rust 测试验证旧配置迁移不丢网格、来源与过滤，非法坐标/选择/revision 拒绝，批量布局原子性，复制新 UUID，以及冷启动恢复。新增 CLI/HTTP/SSE 进程场景覆盖负数/小数坐标、多个矩形一次提交、隐藏/锁定/层级/字号、共享视口和工具栏、过期 revision、插件重启与实例冷启动，以及删除当前面板后清除选择。

其余 9 项 WebUI 进程测试继续覆盖：小 Core 缓冲之外的早期记录查询；跨 Record/UTF-8/stdout/stderr 分行；固定水位、分页、搜索、上下文与历史曲线；动态流归档；错误 epoch/归档身份；迟启和停止归档、写失败、缺口和后续数据；后台任务并发/取消；共享暂停帧；HTTP 同源与 UDP 帧预算。具体断言见 `quality/tests/v2/webui.py`。

## 实际浏览器

工作台通过 CLI 完成初始编排，再用真实 UI 验证：

- Vue Flow 标题拖动把日志窗口从 `(16,16)` 移到 `(88,40)`；另一浏览器接收相同坐标。拖动未被其他窗口遮挡的边角，将 `704×448` 调整到 `752×480`，后端保存。
- 画布「适应全部」、平移、缩放与选择写回 Rust；CLI 原子编排四个面板，两窗口同步位置、尺寸、字体、侧栏与视口。界面置顶后 z-index 为 3。
- 属性栏保留未提交草稿；另一 CLI 修改字号后，旧草稿提交被 revision 冲突拒绝。点击还原后读取新配置。恢复来源选择显示正确 alias，不因 JSON 字段顺序误标等待。
- GridStack 网格兼容页实际从 `(0,0,6,5)` 拖动并缩放到 `(2,1,7,6)`，另一浏览器收到相同布局。
- AG Grid 日志页最多 200 行，DOM 仅保留有限可视行；字体/行高、历史/实时和列配置继续可用。历史扫描在 Core 仅保留 32 Records 的条件下读回运行起始记录，固定 4,106 行边界并翻到 offset 200，然后返回实时。
- ECharts 真实 Canvas 绘图，拖动时间滑块将温度曲线 zoom_start 保存为 `26.08695652173913`，画布视口保持不变；窗口缩放由 ResizeObserver 调整图表。
- 实际检查浅色、深色、属性面板以及 640/390 px 窄窗口；两种宽度都无文档横向溢出，390 px 工具栏最右边界 382 px，属性栏可打开/关闭。曲线刻度在小面板下避免重叠，深色滚动条与坐标标签保持可读。
- 插件重启后，浏览器加载嵌入资源并恢复 Page、模式、位置与显示配置；日志仍只绑定本次 Core。最终浏览器没有 console warning/error。

证据保存在仓库忽略的 `quality/artifacts/webui-workbench-20261002/`：CLI 编排脚本、配置、SQLite 归档、`evidence.json`、浅色/深色截图。演示来源为合成数据；验收后停止生成器，保留本地工作台与有限实时缓存供查看，URL 通过该目录 state.json 对应的 `webui web url` 获取。

## 边界

使用成熟 Element Plus、Vue Flow、GridStack、AG Grid Community 和 ECharts；没有手写布局或图表引擎。构建资源按组件分包，随 Rust 二进制离线提供；未进行整机断网试验。显示配置仍由 Output SQLite 管理，Core 和 output-file 职责不变。这份记录是本机验收，远端各平台结果以该提交 GitHub Actions 为准；真实串口硬件、长时间性能与断电耐久不由本次结果推导。

更早实现记录保留在 [VERIFICATION.v1.md](VERIFICATION.v1.md) 和 [SUSTAINED.md](SUSTAINED.md)，不作为本次证据。
