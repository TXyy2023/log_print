# 视频演示

这两段录像使用 **log_print 0.1.2 / log-print/2**，在 macOS 上实际运行 CLI，再将实时命令和输出记录成视频。脚本驱动流程，日志是专门创建的教学输入；没有调用 AI 模型来扮演操作过程。

## 文件采集与读取

![文件采集演示封面](assets/demos/01-file-read.png)

创建静态教学文件，用 input-file 的 static 模式导入，再查询 Core 分配的流 UUID，读取当前保留的日志快照，核实来源 EOF 并不表示下游完成，最后停止本次实例。

[打开或下载 MP4](assets/demos/01-file-read.mp4) · [对应快速开始](quickstart.md)

## 程序输出、转换与归档

![程序转换与归档演示封面](assets/demos/02-transform-archive.png)

采集教学程序的 stdout 与 stderr，用转换插件发布带编号的派生流，同时保存 JSONL 和 SQLite，再读取归档核实结果。Core 的有界内存并不替代输出插件的保存功能。

[打开或下载 MP4](assets/demos/02-transform-archive.mp4) · [程序采集](guides/program.md) · [转换](guides/transform.md) · [保存](guides/archive.md)

## 复现与验证范围

录制脚本、命令输出和结果校验的组织方式见[素材来源](assets/demos/SOURCE.md)。视频在 macOS 录制；项目跨平台回归的范围与视频展示范围分别记录，视频本身不证明每个平台、每种故障或长期运行均已验证。

标题图为生成的项目视觉素材，与运行输出独立，见[图片来源](assets/branding/SOURCE.md)。
