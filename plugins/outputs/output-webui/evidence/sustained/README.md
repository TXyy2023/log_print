# 持续测试精选证据

2026-09-08，模拟数值经过真实软件链路，180秒、1000行/秒；解释和限制见[实测报告](../../SUSTAINED.md)。本目录是正式交付证据，不是运行状态目录。

- [report.json](report.json)：实测汇总、19次周期快照、8次session修改、CLI耗时和资源摘要。只将本机工作区绝对路径替换为`<workspace>`，全部数字和数组顺序与源报告一致。
- [plugin_tree.samples.json](plugin_tree.samples.json)、[browser_tree.samples.json](browser_tree.samples.json)：逐进程CPU/RSS原始采样，保持源文件字节不变；两棵树分别计量。
- [browser-start.png](browser-start.png)、[browser-final.png](browser-final.png)：独立headless Chrome首尾真实截图，保持源文件字节不变；不替代之前可见Chrome交互验收。
- [linux-pty-summary.json](linux-pty-summary.json)：指定TTY尺寸修复后Linux六组实际回归的结果与环境摘要，省略命令路径、运行state地址和临时URL。
- [MANIFEST.json](MANIFEST.json)：每个数据文件的大小、交付SHA-256、源SHA-256与变换说明。

已检查：没有鉴权token、运行state、Chrome profile或私有真实日志。输入是验收脚本生成的温度/负载数值；进程样本只含本次自建树的PID、角色和资源值。本机完整临时运行目录仍受Git忽略，不应整体加入提交。
