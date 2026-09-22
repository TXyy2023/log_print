# 输入/输出样例和验收

本目录的 Python 程序与文本提供演示输入，不代表真实软件日志。从仓库根目录运行正式验证：

```sh
python3 quality/run.py
```

完整入口会构建当前 workspace，检查协议、CLI 生命周期及输入/输出插件。若已执行 `cargo build --workspace --locked`，可针对性运行 `python3 quality/tests/v2/inputs.py` 或 `python3 quality/tests/v2/outputs.py`。输入检查覆盖 stdout/stderr 原始字节、源进程树关闭、文件静态读取、追加/截断/替换及隔离 tmux；输出检查覆盖显示、派生转换及 raw/JSONL/SQLite 保存读回。测试使用明确标记的合成输入、临时目录和自有子进程，不替代真实硬件或其他平台验收。

语言接入直接配置程序可执行文件与参数。例如 Python `python3 -u script.py`、Node `node script.js`、Java `java -jar app.jar`、已编译 C/Rust 可执行文件等均使用同一个 input-program，不需要每语言插件。只有实际运行记录才计入平台/语言支持验证；参数示例不意味着环境已安装。

0.1.2 使用 `input-file` 的 `mode:static` 读取静态文件，已取消独立 input-replay。串口、TUI、WebUI 暂缓，不参与当前 workspace；当前归档插件仅创建新目标，不提供实时 resume。完整测试要求与平台边界见 [测试说明](../../../quality/README.md)，保存语义见 [output-file](../../plugins/outputs/output-file/README.md)。
