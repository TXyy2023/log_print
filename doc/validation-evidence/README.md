# 可克隆的验收摘要

这些 JSON 保留实际报告中的版本、环境、阶段返回值与结果；完整命令日志位于文档记录的本地 artifacts 目录或对应 GitHub Actions 附件。

- `macos.json`：e0b9203 的本地 self-hosted 完整10阶段。后续5279e8d将同义Core循环改为zip以满足Rust1.98 Clippy，并改进协议测试端的有界缓冲读取；该版本本地11组协议再次通过。
- `linux.json`：5279e8d 的独立Linux ARM64完整10阶段。语言实际5通过、Node/Go因缺工具明确跳过；串口仅PTY。
- `cloud.json`：a0be668的[最终三平台CI](https://github.com/TXyy2023/log_print/actions/runs/34150338258)，逐平台完整10阶段、实际工具版本、语言/IO/UI结果、跳过项及原始报告哈希。与本地汇总逐字节相同，SHA256为`a381a4f75c0850de80bddcee3b83681090d9e9366b52fdb875fb765255994b22`。
- `quickstart.json`：f5a3a23最终release的README与Skill两轮干净目录示例。保留预期/实际字节、二进制与文档哈希、进程及清理结果；完整原报告哈希可核对。
- `dashboard.json`：临时目录里的dashboard示例，双曲线、CLI revision修改、PNG导出及停止流程。

记录的 Git SHA 或二进制哈希标识真正被测版本，不用最后文档提交冒充所有测量的源码版本。原始性能结果另见 `doc/benchmarks/`，三分钟UI/浏览器记录位于 `plugins/outputs/output-webui/evidence/sustained/`。
