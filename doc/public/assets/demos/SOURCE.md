# 演示素材来源

两段演示由本项目自行录制：脚本实际调用 log-print 0.1.2，Playwright 录制本机浏览器实时事件视图，再以 FFmpeg 编码为 H.264 MP4。没有调用模型来扮演执行者，也不表示人类手动操作录屏。

| 文件 | 内容 | 输入来源 |
| --- | --- | --- |
| `01-file-read.mp4` / `.png` | 文件采集、查询 UUID、读取快照、停止实例 | 脚本生成的三行教学文件 |
| `02-transform-archive.mp4` / `.png` | 程序 stdout/stderr、独立派生流编号、JSONL + SQLite 归档 | 脚本生成的教学 Python 程序 |

封面直接取自对应实时事件视图。画面持续标明“脚本驱动实际 CLI · 教学输入”，本机路径替换成具名占位符；长输出面板显示可容纳的末尾行并标注实际行数，完整 stdout/stderr 和命令保存在仓库。文件、流 UUID、退出结果、归档记录与数据库完整性均来自实际运行。

复现脚本、路径替换规则、二进制校验值、逐项断言和 MP4 参数见仓库的 [quality/demos](https://github.com/TXyy2023/log_print/tree/dev/quality/demos)。这些素材由项目创建，按仓库许可使用。视频无音频，无第三方视频或音频素材。
