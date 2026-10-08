# log-print/1 历史实现与验收

这些文件来自 0.1.1（c61c8d7），仅保留历史源码与测试溯源，不属于 0.1.2 工作区组件或当前验收。原测试依赖旧目录、协议和已取消的保存语义；如需复现，请检出 `ver-0.1.1`。

- `components/`：已取消的 io-plugin-util、input-replay、log-plot。
- `tests/`：旧协议、进程、I/O 和归档验收。

0.1.2 的正式检查从 `quality/run.py` 调用 `quality/tests/v2/` 与当前 Rust 测试。

`benchmarks/benchmark.py` 使用 log-print/1 协议及旧存储/绘图配置，仅保留历史测量工具，不兼容当前版本。
