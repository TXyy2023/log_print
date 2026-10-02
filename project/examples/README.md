# 示例

在仓库根目录运行；`basic.json` 跟随 `example.log`，预配置一个可后启动的终端 Output。先创建文件，再 `log-print start --config project/examples/basic.json`；使用 `streams` 返回的 UUID 读取或关联。

`io/` 中的 Python 源和文本是生成的演示输入，不代表真实软件日志。所有示例都受 Core 有界内存及 UDP 尽力发送限制。

## WebUI

`log-print start --config project/examples/webui.json` 启动文件采集、本地 WebUI 和本次 SQLite 归档。用 `log-print webui web url` 获取地址。页面配置独立持久化，浏览器与 CLI 同步；见 [WebUI 手册](../plugins/outputs/output-webui/README.md)。
