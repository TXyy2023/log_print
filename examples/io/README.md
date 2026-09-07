# 输入/输出样例和验收

运行 `cargo build --workspace` 后，`python3 examples/io/verify.py` 用真实 Core 和插件子进程检查 stdout/stderr 无换行字节、源进程树关闭、文件追加/截断/替换、透明 replay、跨块派生转换。测试只创建临时目录及自己拥有的子进程，清理后输出 JSON。它是插件集成检查；不替代主程序 CLI、UI、真实硬件或其他平台验收。

语言接入直接配置程序可执行文件与参数。例如 Python `python3 -u script.py`、Node `node script.js`、Java `java -jar app.jar`、已编译 C/Rust 可执行文件等均使用同一个 input-program，不需要每语言插件。只有实际运行记录才计入平台/语言支持验证；参数示例不意味着环境已安装。

每插件 README 列出配置、缺口/重启/硬件边界及成熟依赖官方来源。serial `--list` 只列出设备，不代替打开采集验收。不要对未知串口设备发送 TX。
