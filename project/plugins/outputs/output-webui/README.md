# output-webui

单一的本地日志阅读界面：Vue 前端、Rust/Axum 后端，离线资源嵌入二进制。支持来源切换、实时跟随、暂停显示、字面筛选与高亮、历史分页、所选日志详情、上下文、文件导入和持续保存。旧工作台前端与备用入口已移除。

## 启动

```sh
# 空实例，直接在浏览器导入日志文件
log-print start --output-webui web
log-print webui web url

# 跟随一个文件
log-print start --input-file source=./app.log --output-webui web

# 配套 SQLite 归档，提供超过内存缓存的历史
log-print start --input-file source=./app.log --output-webui web --webui-archive web=./capture

# 绑定同实例已配置的 output-file SQLite 目标
log-print start --input-file source=./app.log --output-sqlite archive=./capture/run.sqlite \
  --output-webui web --webui-history web=archive
```

归档参数互斥，不覆盖已有文件。默认监听 `127.0.0.1:0`。可设置 `listen`、`state_path`；`history_path` 和 `runtime_id` 由 supervisor 生成。

## 阅读与定位

- 来源栏区分实时流与导入文件。大文件通过 Rust 后台查询，浏览器只显示有限数据。
- 暂停只冻结显示，采集和保存继续。实时窗口最多 2,000 行；滚动离开末尾后可一键回到最新。
- 关键词按字面匹配，支持只高亮或仅显示匹配。Enter/Shift Enter 在当前窗口或当前历史页内定位；仅高亮不重新扫描历史。
- 点击日志后可复制文本、打开详情、读取前后 8 行上下文。位置列使用 Record 序号与字节偏移，不将一个 Record 冒充一行。
- 历史查询固定读取边界，每页最多 200 行。时间列是采集时间，不是从原文推断的来源时间。缺口、归档故障与查询失败明确显示。
- Ctrl/Cmd F 查找；日志列表支持方向键、PageUp/PageDown、Home/End，Enter 打开详情；Space 暂停/继续，Esc 关闭浮层。浅/深色模式、选中来源保存在本浏览器。

## 导入与保存

上传最多 64 MiB，要求 TCP supervisor。每个文件由独立 input-file 进程发布到 Core，再由 output-file 的 Archive 实现保存 raw 与 SQLite；输入按检查点确认限制在途数据。只有源 EOF、字节数相同且输出无缺口时才报告完成。导入完成后自动显示完整的可查询历史，失败保留上传原件。

“保存日志”提供两个明确起点：

- **从现在开始保存**：固定后端处理操作时的 Core head，保存后续原始日志。
- **从最早可用数据保存**：补入可读归档、WebUI 缓存和 Core 数据，去重后继续记录新增内容。不可恢复的缺口不冒充完整历史。

“保存记录”中停止并提交后才能下载 `.log`。暂停、筛选与高亮不影响原始保存内容。文件位于 WebUI 状态目录 `recordings/<UUID>/`；文件保留在磁盘，运行内列表和导入流不在 supervisor 重启后自动重建。最多同时 4 个保存任务，每轮最多 64 个任务/导入流。

## 工程与接口

前端运行时依赖仅 Vue。组件和设计系统见 [DESIGN.md](DESIGN.md)。浏览器的查看状态独立；Rust 的共享配置和查询 API 继续供 CLI/TUI 使用，不代表旧画布界面仍存在。

- `GET /api/state`、`GET /api/events`：来源、导入、保存与连接状态。
- `POST /api/control`：`logs.tail`、`history.read/search/context`、`query.get/cancel`、`recording.start/stop`。
- `POST /api/import`：原始 body，URI 编码 `X-File-Name`。
- `GET /api/recordings/<id>/download`：已停止且成功提交的日志。

写接口检查 Host/Origin，凭据只保留在 Rust 进程。Core 的滚动缓存与磁盘归档是不同范围；实时显示不是已保存证明。增量分行缓存每流最多 10,000 行或 2 MiB、总计 32 MiB，另计原有 Record 缓存。

## 构建与验收

```sh
npm ci --prefix project/plugins/outputs/output-webui/frontend
npm run build --prefix project/plugins/outputs/output-webui/frontend
cargo build --workspace --locked
cargo run --locked -p log-print-quality --
cargo run --locked -p log-print-quality -- browser
```

`assets/` 是提交并嵌入的离线产物。浏览器验收使用真实 Rust 服务，覆盖 1440/1280 桌面与 390px 移动视口、查询/上下文、键盘/焦点、暂停、两种保存的逐字节校验、导入、错误与恢复。截图、trace 和 `browser.json` 保存到 `quality/artifacts/browser/`。历史验收记录不代表本次界面已通过跨平台验收。
