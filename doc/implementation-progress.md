# 正式版本实施进度

开始：2026-09-08。用户已授权执行整个 `todolist-structured.md`，通过 `/goal` 持续推进。

## 已完成的基线工作

- 实施前实际工作区完整压缩归档至 `local-archive/pre-formal-2026-09-08/full-worktree.tar.gz`，含原根 Git、嵌套插件 Git、未提交源码、文档、运行资料、构建缓存。
- 逐项验证 7,487 个文件或链接，压缩包大小 353,605,162 bytes，SHA-256 `310ffdfea9bf8e4fd999d710f545cc152c246d31a12d7c7eeb34bee298070ef9`。首次核验未处理 tar 硬链接，修正核验方法后通过；没有先删除再归档。
- 原实现完整移至 `local-archive/mvp-v2/worktree`，原 v1 快照复制至 `local-archive/mvp-v1`。本地归档统一 Git 忽略，原始 `doc/todolist.md` 保持原字节。
- 盘点时旧主程序内嵌 Core，Rust SDK 为旧接口/占位，旧 Python storage 并非本版要求；正式版本已按新职责重新实现。
- 原根 Git 历史只有初始化提交 `66d5e51`，含 MVP 骨架。保留历史，当前实施分支 `codex/formal-v1`。开工时指定GitHub仓库为空；交付只提交正式源码及文档，不提交本地归档，也不清洗原历史。

## 已实现与已验证

- 一个主进程管理唯一Core及独立插件；loopback TCP、log-print/1协议与Rust SDK共用同一workspace。SDK完整帧发送、取消和背压已回归。
- 程序、文件、串口只读、Replay四类Input；raw、转换、TUI、WebUI四类Output；多流/派生DAG、CLI、独立session与导出已实现。
- Core有界缓冲和可选SQLite分段保存，统一历史/live接口；保存失败阻塞单流并支持人工恢复。独立故障审查修复缺段、损坏、反馈环和握手清理问题。
- 本地Mac与独立Linux ARM64均完成包含语言验证的完整10阶段；Mac七种语言真实程序字节校验通过，Linux容器五种通过、Node/Go缺失明确跳过。云CI及逐平台差异见验收报告。
- Chrome实际操作和POSIX PTY已验收，图表导出已实际查看。180秒、每秒1000行的双UI持续测试完成，浏览器资源与插件资源单列；限制见正式报告。
- 两轮实际端到端/资源基准，raw、保存、TUI各12000条逐字节核对；保留所有原始结果和测法限制。
- 归档已抽样独立恢复8类文件，哈希一致；原始todo仍为SHA-256 `d5c07c5b6d56f458f5dc89d192c42936aaf67b37186dda022739d61cc3161e7c`。

## 交付收口

README与Skill两套教程已在干净临时目录、最终release构建上完整运行，均精确读出17字节并回收全部实例进程。dashboard示例完成双曲线、CLI修改、导出与停止。代码已推至验证分支；三平台云CI与最终Git交付以[验收报告](validation.md)及远端提交为准。

真实串口硬件、可见终端窗口、掉电/坏盘、长时间运行上限分别保留未验证边界。旧版本通过项没有计入本版验收。
