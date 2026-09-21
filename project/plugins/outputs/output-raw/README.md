# output-raw 0.1.2

终端展示插件，配置 `{"streams":["source"],"annotate":false}`。
默认把订阅记录 payload 原字节写到 stdout；多流按本插件收到记录的顺序交错，不声明全局时间排序。
`annotate:true` 在每条记录前显示流 ID、Core 接收序号和来源通道。
路径、文件保存、历史起点和动态配置不属于本插件；交给 output-file 保存。
订阅从 Core 当前仍保留的最早记录开始，读到末尾后持续等待，手动停止退出。
