# log_print 正式版本协作

先读 README.md 和 doc/README.md。当前协议是 log-print/1，正式需求及未覆盖项在 doc/todolist-structured.md 与 doc/validation.md。

- 主程序/Core/协议/SDK在 crates/，官方插件在 plugins/。核心库不能依赖具体插件。所有官方包同一 workspace。
- Core和插件均为主进程的子进程。跨插件数据经过Core；业务解析、转换、图表在插件。原始流与派生流独立。
- 修改协议需同步类型、语言无关规范、SDK、插件和实际进程验收。保存确认必须对应持久提交，不得把收到请求或未知结果改写为成功。
- 队列/帧/负载/图表窗口有界；慢端、取消、断开、覆盖和写失败须可观察。优先复现已有脚本中的失败，再作修改。
- `python3 tests/run.py` 是完整验收入口。单包/单用例仅作迭代；真实硬件、实际浏览器、模拟数据、容器和云平台覆盖分开记录。
- 仅回收测试/实例自己创建的进程。按需启停不引入自动重启watchdog、硬件TX、多Core汇流或云托管。
- local-archive/ 是用户本地旧版本，不改写归档、不提交归档、不清洗现有Git历史；doc/todolist.md 是原始清单。
