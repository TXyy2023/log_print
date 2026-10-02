# output-webui 验收记录 — 2026-10-02

本版使用 log-print/2，Rust/Axum 后端和内嵌 Vue 前端。此次执行环境为 macOS 26.5 arm64、Rust 1.92.0、Python 3.14.1；实际浏览器为 Codex 内置浏览器的两个独立窗口。来源为明确标注的合成 temperature/voltage 数据。

## 自动检查

最终入口：`python3 quality/run.py --report-dir quality/artifacts/validation/output-webui-completed`。机器报告位于该目录的 `results.json`，12 个步骤全部通过，`complete_selected_suite=true`。

- 前端锁文件安装、Vue/TypeScript 类型检查、许可证汇总和离线构建通过；npm 安装审计未发现漏洞。
- workspace 格式检查、Clippy 全目标零警告、77 个 Rust 测试和全部二进制构建通过。
- 真实进程验收共 62 项：协议 13、supervisor 8、启动故障 4、CLI 5、输入 11、输出 12、WebUI 9；未跳过项目。
- 文档公开入口构建通过；22 个公开页面的 769 个链接目标检查无错误。

WebUI 的 9 组进程测试覆盖：

1. 默认无归档、HTTP/SSE、Origin 拒绝、revision 冲突、空流及小缓冲覆盖状态。
2. 420 条日志超过 Core 缓冲后的早期记录分页、全文/正则搜索、跨 Record UTF-8 和 stdout/stderr 分行、前后上下文、历史曲线、精确纳秒边界、固定查询水位、动态派生流归档、错误 epoch/归档身份拒绝及停止归档状态。
3. 迟启归档的实际起点和未覆盖前缀，注入 SQLite 写失败后的真实故障与尚未提交范围。
4. UDP 回复帧预算、大批结果分页、按 Record 字节偏移定位上下文；UDP 流目录描述超过单帧时完整分页获取。
5. 两个后台任务并发限制、第三个拒绝、取消、任务分页及有界实时缓存。
6. CLI 独立编排 Page/面板/曲线、列宽/排序/暂停、缺失来源保留、插件重启和实例冷启动恢复；实时、配套归档、绑定显式 SQLite 三种启动方式；互斥参数和不覆盖现有归档。
7. 慢归档、Core 单条缓冲及队列溢出产生持久缺口后继续归档后续记录；日志和经过通道/文本/时间过滤的曲线保留缺口。
8. 暂停帧由 Rust 后端共享，新增查看者得到相同帧；暂停期间 Core 持续采集，恢复显示后显示最新记录。

部分条目共享一组测试；编号表示覆盖场景，不表示额外测试数量。

## 实际浏览器

- GridStack 官方 Vue 集成：实际拖动窄面板到另一列，实际将日志面板高度 6 改为 7，第二窗口同步接收布局；CLI 恢复布局、插件重启后恢复选中 Page 与布局。
- AG Grid Community：200 行数据页只渲染有限可视行；实际把时间列从 120 拖到 160 像素，SQLite 提交后第二窗口同步，后续 SSE 更新没有重置列宽。
- ECharts：真实 Canvas 绘制两条曲线，滚轮缩放提交到 Rust 后端；历史曲线扫描 46,924 Records 后共返回 1,124 个点，低于两曲线合计 2,000 点预算。
- CLI 创建/选择 Page 和修改标题均同步到两个窗口；在旧表单打开期间用 CLI 改标题，旧表单保存被 revision 冲突拒绝。
- 真实暂停/继续操作：两个窗口保持相同冻结尾行，采集继续；新窗口和 CLI 读取同一后台暂停帧。
- Core 仅保留 8 条 Record，仍可浏览起始序号 1 的归档、翻到下一页、双击历史行取得 21 行上下文，并绘制本次运行的历史曲线。分页结果包括真实缺口标记，因此显示行数可以超过扫描 Record 数。
- 输入 `temperature=26.` 后立即点击全范围搜索已实际复测，首个结果页全部匹配；查询等待筛选配置成功提交，不使用旧条件。
- 最终构建对应浏览器未发现控制台 warning/error。资产均由本地服务提供，不依赖运行时 CDN；未执行整机断网试验。

浏览器证据位于仓库内忽略的 `quality/artifacts/webui-browser/`：`evidence.json`、`history-context.png`、`resized-history.png` 和 `final.png`。演示输入停止后保留 Core、归档和 WebUI 供本地查看；其状态文件为 `quality/artifacts/webui-demo/state.json`，URL 可通过 CLI 的 `webui web url` 获取。

此次结果证明本机功能链路与故障边界，不是 Windows/Linux 实测、真实硬件采集、断电耐久或性能基准。旧实现记录保留在 [VERIFICATION.v1.md](VERIFICATION.v1.md) 和 [SUSTAINED.md](SUSTAINED.md)，不作为本版证据。
