# output-webui：页面与历史

本地 Rust/Axum 后端通过当前 SDK 读取 Core。前端使用 Vue 3、Element Plus、GridStack 官方 Vue 集成、AG Grid Community 和 ECharts，构建产物嵌入 Rust 二进制，运行无需 Node、CDN 或互联网。

```sh
log-print start --input-file source=./app.log --output-webui web
log-print webui web url
```

默认实时模式不承诺全量历史。需要完整上下文时，在启动命令加 `--webui-archive web=./capture` 创建本次全流 SQLite 归档，或用 `--output-sqlite archive=./capture/run.sqlite --webui-history web=archive` 绑定同实例显式归档。两种方式互斥，不覆盖已有文件。

## Page 与 CLI

```sh
log-print webui web page create --name monitor --title '运行监控'
log-print webui web page select --page monitor
log-print webui web panel add --kind log --title Logs --stream source
log-print webui web panel set --panel PANEL_UUID --text error --metadata true --paused false
log-print webui web panel add --kind curve --title Temperature
log-print webui web series add --panel CURVE_UUID --name temperature --regex 'temperature=(?P<value>[0-9.]+)'
```

页面配置、布局、过滤规则、曲线定义和当前页面独立存入 SQLite。拖拽/缩放、列设置、暂停、图例、曲线缩放均由 Rust 后端管理，CLI 与全部浏览器同步。网页提交携带 revision，提交成功后确认；CLI 可用 `--revision` 做并发检查。

来源绑定保存 owner/alias，在本次 Core 解析 UUID；缺失来源显示“等待来源”。冷启动恢复配置，重新绑定当前运行，不读取上次运行的日志。支持日志多流、通道、文本/正则过滤和文本/十六进制显示；曲线支持多条定义、正则命名捕获组 value 或 JSON 字段路径、时间范围、坐标范围、颜色及线宽。

## 全量上下文与覆盖边界

```sh
log-print webui web history search --stream source --regex ERROR
log-print webui web history context --stream source --epoch EPOCH --seq 123 --byte-offset 20 --before 10 --after 10
log-print webui web history curve --stream source --field metrics.temperature
log-print webui web query get --query QUERY_UUID --offset 0 --limit 200
log-print webui web query cancel --query QUERY_UUID
```

历史命令返回任务 ID。查询固定 SQLite 提交水位和 Core epoch/head，合并、去重后分页，每条结果保留 Record 身份和字节偏移；实时和历史共用跨 Record 分行与数值提取逻辑。上下文支持独立 stdout/stderr。最多两个后台扫描同时运行，日志页最多 200 行，历史曲线最多 2000 个绘制点。CLI 还受 TCP/UDP 帧预算限制。

界面和 CLI 明确返回归档起点/提交水位、内存范围、未覆盖前缀、缺口、尚未提交和归档故障。停止归档后仍能读取已提交前缀。没有归档时，历史查询只使用 Core 当前内存。新 SQLite 归档为 schema 3，只读兼容 schema 2；原 raw/JSONL 格式不变，已有档案不自动迁移。Core 不访问数据库。

默认每流缓存 4096 Records 或 4 MiB，总记录缓存 64 MiB。浏览器只保留有限结果页；暂停显示仍继续采集。查询临时文件与 Page 数据库分开，重启清除临时定位。

完整命令见 [CLI 参考](../reference/cli.md)，JSON 字段见 [配置参考](../reference/configuration.md)，归档原有设置见 [output-file](output-file.md)。
