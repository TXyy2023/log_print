# 双 UI 持续负载与浏览器资源记录

2026-09-08，macOS arm64，release 构建；实际 Chrome 152.0.7977.82 使用独立临时 profile/headless 模式。来源为按时间节奏逐行写入文件的独立 Python 进程，经过正式 input-file→Core→TUI/WebUI 链路。TUI 写入持续排空的真实 POSIX PTY；浏览器打开真实本地页面，通过 CDP 检查 Canvas、series 和动态 revision。此项与此前真实可见 Chrome 交互、实际可见 Terminal 验收分开。

运行 `sustain_ui.py --duration 180 --rate 1000`，实际写入 180000 行 / 179.9995 秒。每个插件两个独立 session，每 session 两条数值曲线；刷新配置为100ms，保留上限2048点/曲线、显示上限512点/曲线。Core使用默认4MiB/4096条缓存，SDK事件队列容量64条；没有开启持久保存。

结果：两个插件的四个 session、八条曲线均匹配180000行，完整行、合法数值计数吻合，gap/断连/非法UTF8/超长行均为0，最后无残留半行。19次周期采样中保留点与显示点均未突破配置界限。累计PTY输出242433字节。每20秒轮流修改Web/TUI alpha，共8次；所有beta始终revision=1且配置不变，另一个插件的alpha revision保持独立。Web端每次修改的revision都由实际浏览器读回确认。

CLI session命令从进程启动至收到返回的耗时为p50 **7.46ms**、p95 **8.67ms**、最大 **14.46ms**。这是CLI控制往返，未把它解释为采集到屏幕扫描的端到端延迟。页面的100ms是刷新配置；每10秒观察到持续变化的generation和实际Canvas，不将这一采样频率写成精确屏幕FPS。

| 范围 | CPU（单核100%口径） | 峰值RSS |
|---|---:|---:|
| 管理器 + Core + input-file + TUI + WebUI | 17.06% | 33.92MiB |
| 其中 Core | 7.19% | 8.75MiB |
| 其中 input-file | 6.89% | 4.09MiB |
| 其中 TUI | 1.58% | 7.78MiB |
| 其中 WebUI | 1.38% | 9.02MiB |
| 独立 Chrome 整棵进程树 | 9.44% | 1384.09MiB |

RSS为同一棵树各进程RSS之和，可能重复计算共享页，不代表系统物理内存净增量；各组件峰值未必同时发生。CPU测量从页面首次就绪后开始，不包含Chrome启动。两棵树由两个独立psutil采样器约100ms采样，分别得到1607/1604个样本；采样器各消耗约14 CPU秒，单列且不包含在上述树内。独立数值生产器、测量脚本和用户既有浏览器均不属于插件树。宿主整体CPU均值约14.7%，本次不是封闭的专用基准机测量。

30秒后的插件树RSS从30.48到33.92MiB，线性拟合约+0.77MiB/分钟；Chrome树从1111.23到1250.50MiB，线性拟合约+8.12MiB/分钟（过程有波动，首尾差不等于拟合斜率）。**本次没有证明浏览器内存已达到稳定平台**，也不足以将RSS增长认定为内存泄漏。没有从三分钟结果推断数小时稳定性或最大可持续输入速率。

有界性代码复核：业务JS只初始化一个ECharts实例，当前快照通过赋值替换；`setOption`使用`notMerge:true`且关闭动画，每条传入曲线最多512点。曲线DOM通过`replaceChildren`替换；切换session先关闭旧EventSource，页面卸载关闭当前连接，只有一个ResizeObserver。服务端SSE最多16连接，每个连接按消费者拉取产生当前快照，不建立持续追加的事件队列，断开/停止会释放连接permit。共享数值环形缓冲在入流、gap和断连路径都裁剪点数。未找到业务层明确的无限数组、反复初始化chart或未关闭订阅；高频setOption对象分配、Canvas和Chrome缓存仍需更细的堆/浏览器分析才能归因。没有为了消除增长数字而调用GC后替换本次实测结果。

传输队列**实时占用未由接口暴露**，因此报告保留了固定容量、数值ring点数和完整行进度三种不同信息，不用推算积压冒充实际队列测量。2048点上限也意味着1000行/秒时只保留最近约2.05秒数值，即使横轴窗口设为20/60秒；这是本次选择的有界保留策略，不声称保存整个时间窗的全部点。

复现入口：[sustain_ui.py](tests/sustain_ui.py)。[原始报告](../../../artifacts/ui/sustain-180s-1000lps/report.json)、[插件树样本](../../../artifacts/ui/sustain-180s-1000lps/plugin_tree.samples.json)、[浏览器树样本](../../../artifacts/ui/sustain-180s-1000lps/browser_tree.samples.json)、[结束时浏览器截图](../../../artifacts/ui/sustain-180s-1000lps/browser-final.png)。所有本次应用、生产器和独立Chrome进程已停止；state与临时Chrome profile已删除。截图已实际打开检查，显示正常双曲线、4096保留点、零非法值和零缺口。

后续针对Windows detached控制台信号注册失败加入Err→pending处理后，fmt、严格clippy及macOS六组真实进程/PTY回归再次通过；这项退出路径修复未更改绘图或数值热路径。Linux指定PTY的尺寸查询修复也已有实际六组回归证据，见 [Linux PTY报告](../../../artifacts/linux/validation/formal-linux-pty-fixed-7e9c/report.json)。平台正式全套结果由根平台报告单独记录。
