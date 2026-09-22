# 0.1.2 实际 CLI 演示

这里保存两段 README 演示的驱动、真实运行输出及校验结果。视频是浏览器实时显示脚本执行事件的录像；由脚本调用本仓库实际二进制，不包含新 Codex/其他模型调用，也不是人类操作屏录。`teaching.log` 和 `teaching.py` 是明确标注的教学输入，不是生产软件日志。

| 演示 | 内容 | 断言 |
| --- | --- | --- |
| `file-read` | `input-file/static` → UUID → `read --raw` → `stop` | 5 项：版本、原字节一致、EOF 不声称下游完成、state 删除、自建进程退出 |
| `program-archive` | stdout/stderr → 原始流 → 独立编号派生流 → JSONL + SQLite | 13 项：源程序退出、通道、原流不变、派生 UUID、编号、归档确认、元数据、SQLite 完整性及实例退出 |

视频和封面位于 [`doc/public/assets/demos`](../../doc/public/assets/demos/)。编码为 H.264/yuv420p、1600×900、25 fps，无音轨，启用 MP4 faststart。录制留有阅读停顿，未加速播放。终端面板保留原始输出，超过面板高度时明确标注可见末尾行数；完整命令与 stdout/stderr 见下面的 JSONL。

## 可追溯证据

- [`evidence/file-read/transcript.jsonl`](evidence/file-read/transcript.jsonl)、[`assertions.json`](evidence/file-read/assertions.json)
- [`evidence/program-archive/transcript.jsonl`](evidence/program-archive/transcript.jsonl)、[`assertions.json`](evidence/program-archive/assertions.json)
- 每次运行的 `config.json`、教学输入、`recording.json`、`ffprobe.json`；程序演示另保存实际生成的 `numbered.jsonl`。

`transcript.jsonl` 包含界面未显示的就绪轮询（`visible:false`）。命令参数和原始输出只替换明确的本机路径：构建目录为 `<bin-dir>`、独立临时目录（含 macOS 规范化路径）为 `<demo-dir>`、仓库为 `<repo>`、Python 解释器为 `<python3>`。UUID、PID、时间、序号和日志字节保留实际值。没有收集/发布私有运行时配置或认证 token；脱敏的静态 `config.json` 是证据，需要把占位符换成真实路径后才能单独使用。

录制基线为 `7a3ab38d4651c3c44be60efac2ccac613172a2f8`（0.1.2）。真实二进制 SHA-256 记录在 transcript 的 `environment` 事件中。

## 复现实际流程

仓库根目录构建后，用 Python 标准库即可运行两套断言，不依赖浏览器：

```sh
cargo build --workspace --release
python3 quality/demos/run_demo.py file-read \
  --bin-dir "$PWD/target/release" \
  --output quality/artifacts/demo-check/file-read
python3 quality/demos/run_demo.py program-archive \
  --bin-dir "$PWD/target/release" \
  --output quality/artifacts/demo-check/program-archive
```

当前进程退出检查使用 POSIX `ps`；该演示工具面向 macOS/Linux。源程序与插件的 Windows 产品验收由 `quality/tests/v2` 覆盖，本脚本不声称跨平台录制。每次执行生成独立临时目录，归档始终使用新建路径；只对自己生成的 state 调用 `stop`，并核实观察到的 supervisor/Core/插件进程身份已经退出。成功后删除临时运行目录，保留指定输出目录中的证据。

## 复现视频

另需 Node.js、Playwright（含可运行的 Chromium）、`ffmpeg` 和 `ffprobe`。已经有可用 Playwright 时，通过 `--playwright-module` 指向它的 `index.mjs`；不需要修改本仓库依赖或安装 Homebrew 包。

```sh
node quality/demos/record_demo.mjs --scenario file-read \
  --bin-dir "$PWD/target/release" \
  --playwright-module /absolute/path/to/node_modules/playwright/index.mjs
node quality/demos/record_demo.mjs --scenario program-archive \
  --bin-dir "$PWD/target/release" \
  --playwright-module /absolute/path/to/node_modules/playwright/index.mjs
```

如果当前 Node 模块解析路径中已有 `playwright`，可以省略最后一项。脚本在 `127.0.0.1` 随机端口启动临时事件视图，浏览器录完后关闭；原始 WebM、完整编码日志与最终帧保存在忽略的 `quality/artifacts/showcase-demo-recordings/`。MP4、PNG 与脱敏证据会覆盖本目录对应演示产物，适合显式重新录制，执行前先保存需要保留的旧录像。

本示例只证明有限教学输入在这一轮运行中的结果。Core 是有界内存；`read` 是快照，不是持续游标。source EOF、TCP 接受、无可见缺口，均不等同全流水线完整持久化。程序演示在确认转换停止及归档成功响应后才检查两个归档目标，不把源 EOF 当作保存完成。
