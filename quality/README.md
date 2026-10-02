# 测试与 CI

从仓库根目录运行：

```sh
python3 quality/run.py
python3 quality/run.py --list
```

需要 Python 3.12+、Rust stable（rustfmt / Clippy）、Node.js 22.12+ 与 npm。入口先按锁文件构建离线 WebUI 资源，再执行格式、Clippy、全部 Rust 测试、构建，以及真实进程协议、CLI 生命周期、输入和输出验收。测试生成的数据均为明确标注的 fixture，不冒充真实软件日志。

| 位置 | 验收范围 |
| --- | --- |
| 各 crate 的 Rust 测试 | 协议限额、权限/缓冲、SDK、输入配置、输出文件/SQLite事务及转换窗口 |
| `tests/v2/protocol.py` | TCP/UDP、独立多流/多订阅、覆盖、慢消费者、非法身份和Core重启 |
| `tests/v2/supervisor.py` | CLI真实管道、配置快照、后启动/重启、UUID绑定、拒绝旧配置及清理 |
| `tests/v2/cli.py` | 无配置文件启动、文本结果、命名参数、二进制原始读取、转换和归档读回 |
| `tests/v2/inputs.py` | 静态文件、跟随/截断/替换、双通道程序、异常退出、进程树、隔离tmux |
| `tests/v2/outputs.py` | 显示与保存读回、转换派生流、停止与错误 |
| `tests/webui_frontend.py` | 锁文件安装、许可证生成、Vue 类型检查与 Vite 构建 |
| `tests/v2/webui.py` | HTTP/SSE、Page/CLI 恢复、实时与 SQLite 归档衔接、动态流、上下文与曲线、后台任务和帧预算 |
| `tests/docs/verify_links.py` | 文档站链接与附件 |
| `archive/v1/` | 旧组件/旧协议验收，保留溯源、不作为本版通过证据 |
| `artifacts/` | 本地忽略的报告、完整日志及备份 |

`quality/ci/README.md` 说明报告与跨平台边界。tmux 仅 Unix 环境且已安装时运行，缺失必须记录 skip；本机实测结果单独记录。构建失败不能接受旧二进制，`--skip-build` 仅迭代，不能替代完整检查。

文档：

```sh
npm run build:local --prefix doc/site
npm run build:public --prefix doc/site
python3 quality/tests/docs/verify_links.py doc/site/dist/local
python3 quality/tests/docs/verify_links.py doc/site/dist/public
```

[0.1.3 发布与验收](release-0.1.3.md) · [0.1.2 历史验收记录](release-0.1.2.md)

## 双显示器与真实交互

`quality/run.py` 先构建离线 WebUI，再运行完整 Rust 检查和真实进程测试。构建包含 `output-tui` 及测试专用 `pty_driver` example。`webui-v2` 的十项共享后端契约在 `tui-v2` 再执行一次，覆盖归档三种启动方式、动态流、溢出历史、固定边界、恢复、故障、暂停、资源预算和 revision。

`tui-v2` 同时通过 portable-pty 启动真实终端，Windows 使用 ConPTY，不跳过交互。vt100 只解析实际输出，不模拟应用后端；验证键盘/鼠标、Unicode、终端大小、双视图、并发编辑、历史、断连、退出恢复和无 TTY 快照。

GitHub `validate.yml` 的 Linux/macOS/Windows 每个任务均在全套之后安装 Chromium，执行 `quality/tests/v2/browser.py`：两个真实浏览器、VueFlow 拖拽/缩放、GridStack 兼容缩放、AG Grid 虚拟表格、过滤/历史、ECharts 缩放与配置恢复。`quality/artifacts/browser/` 保存成功或失败截图、trace.zip 和 browser.json，与命令日志一并上传。前端脚本只依赖精确锁定的 Playwright 开发依赖，运行时不需要它。

单独执行浏览器验收：

```sh
cd project/plugins/outputs/output-webui/frontend
npm ci
npx playwright install --with-deps chromium
cd ../../../../..
python3 quality/ci/python-test.py quality/tests/v2/browser.py
```

本地 `quality/run.py` 的成功只证明当前主机的 Rust/进程/PTY 套件；三系统浏览器和跨系统结论应以同一提交的 GitHub job 为准。
