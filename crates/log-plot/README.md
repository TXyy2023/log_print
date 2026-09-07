# log-plot

正式 Output 插件共享的数值提取、会话与导出库。它不直接连接 Core；`output-tui` / `output-webui` 使用 Rust SDK 收取 Record、Gap、Disconnected 后调用此库。

## 提取与有界窗口

每流保留有界跨块分行状态。UTF-8 文本按 LF 分行（去除 CRLF 的 CR），完整行中的正则命名捕获或 JSON Pointer 转换为数值。小数、负数、科学记数法可由规则提取。默认规则取每行的第一个数字；实际带时间戳的日志应明确配置字段。采集/Input/Core 不等待换行，只有数值业务解析等待完整行。未终止尾行保持 pending，不猜测数字是否结束。

- 正则每次配置时编译一次，模式 ≤ 4096 字节，编译/DFA 缓存各 ≤ 2 MiB；必须声明命名捕获，默认 `value`。
- JSON Pointer 例如 `/sensor/temperature`，只接受 JSON 数字；缺失字段记 unmatched，坏 JSON/非数值记 invalid。
- 不接受 NaN、Infinity 或绝对值大于 1e100 的值；后者避免绘图坐标计算溢出。拒绝计数公开。
- 默认每行 65536 字节；超限丢弃该行剩余内容直到 LF，并增加 oversized_lines。非法 UTF-8 行单独计数。Gap/断连清空残片，曲线保留断点。
- 每个 session 1..8 条曲线、16..8192 点/曲线，默认 2048；每插件最多 16 个 session，总配置点数预算 131072。订阅流最多 32，默认每流64KiB分行缓存。
- 每次渲染/导出最多 512 点/曲线。超过时按桶保留极值和断点，不修改 Core 原始数据。窗口时间 0.1..86400 秒，默认60秒。点数和时间都可使早期数据退出视图。
- 横轴为 Core `observed_ts_ns` 秒值，不伪装设备采样时间；墙钟回退会计数并钳制显示时间，保持单流显示顺序。流和 epoch/seq 状态纳入导出元数据。
- 传输断连公开 `disconnected`、`disconnections`、`disconnect_reason`，缺失数量未知，不冒充已知丢失条数。

## Session RPC

两个插件的以下方法均由 `log-print plugin call PLUGIN METHOD --json ARGS` 或友好 session 命令调用。

| 方法 | 参数 / 结果 |
|---|---|
| `sessions` | 无参数；返回 session 配置、revision、点数、逐流健康状态及上限 |
| `session.create` | 完整 SessionConfig，返回初始 revision=1 |
| `session.get` | `{"id":"alpha"}`；有界显示快照，含所用配置、数据范围、source_status |
| `session.patch` | `{"id":"alpha","revision":1,"patch":{"window_secs":10}}`；仅在当前 revision 匹配时原子应用，成功递增 |
| `session.export` | `{"id":"alpha","revision":2,"path":"/tmp/chart.png","format":"png"}`；冻结当前快照，导出并返回路径、范围、revision |
| `config.get` | 返回 effective 顶层配置、当前各 session 配置、动态及重启项 |

SessionConfig：

```json
{
  "id": "alpha", "title": "Telemetry",
  "series": [
    {"name":"Temperature","stream":"sensor","pattern":"temp=(?P<value>[-+0-9.eE]+)","capture":"value"},
    {"name":"Supply","stream":"json-sensor","json_pointer":"/voltage"}
  ],
  "window_secs":60, "max_points":2048, "refresh_ms":100,
  "theme":"dark", "y_min":null, "y_max":null, "paused":false
}
```

`id` 只能为1..64个 ASCII 字母、数字、`-_.`，不能改名；其他上述项通过 patch 动态修改。theme 为 dark/light，refresh_ms 为25..10000。min/max 为可选数值，min必须小于max。变更 extraction 后该曲线从新数据重新累计，不把旧规则点伪装成新规则。未变的曲线保留有界点；扩大窗口不能恢复已淘汰的点。paused 冻结显示，采集和有界留存仍继续；快照的 source_status 保留捕获时来源状态，live_source_status 始终报告当前健康状态，因此暂停不隐藏断连；恢复时显示当前留存窗口。在暂停时改变提取曲线会创建新的显示快照。

## PNG / SVG

Plotters 0.3.7 (MIT)，启用 bitmap_backend、svg_backend、line_series、ab_glyph。PNG 使用 image 0.24.9 编码，不依赖系统字体。`assets/NotoSansSC-Regular.otf` (SIL OFL 1.1) 支持中文标题，来源及哈希见 SOURCES.md；字体约8.3MB，SVG嵌入同一字体以便跨机器打开，因此SVG约11MB起。

默认1200×700，可选 width=320..4096、height=240..4096，最多8Mi像素。CLI同时写 `chart.png.json`/`chart.svg.json`，包含冻结样本、来源epoch/seq、轴范围、视图配置、revision、减采样说明及字体信息。默认拒绝覆盖；`overwrite=true` 才允许覆盖已有图和元数据。保存文件显式检查写入与同步结果。

导出记录请求时实际可用窗口；两种UI和导出使用相同有界样本/配置，终端、ECharts、Plotters的字体布局和像素细节会有差异，不宣称是截图。导出任务不持有入流锁。首版不提供任意历史区间的图形回溯产品。

## 验证

`cargo test -p log-plot` 包含跨块、session隔离/冲突、缺口/重复、限制、暂停和实际PNG解码/SVG字体检查。

真实子进程链路：`python3 plugins/outputs/output-webui/tests/verify_ui.py --pty`（POSIX）。Windows省略 `--pty`，该模式仅验收后端与导出，真实Windows终端另行验证。
