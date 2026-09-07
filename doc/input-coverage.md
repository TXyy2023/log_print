# 实际语言输入覆盖验收

2026-09-08 在 macOS 26.5 / arm64 上运行 [`examples/io/languages.py`](../examples/io/languages.py)：**7 项通过、0 项跳过、0 项失败**。这里的每一项都实际运行了对应语言的程序，并经过 `input-program → Core → output-raw`；编译型语言使用本机编译器生成独立临时可执行文件。

## 本机实测矩阵

| 语言 | 实际运行时 / 编译器 | stdout | stderr | 退出前无换行推送 | 结果 |
| --- | --- | --- | --- | --- | --- |
| Python | Python 3.14.1 | 27 字节一致 | 27 字节一致 | 通过 | 通过 |
| Rust | rustc 1.92.0 (`ded5c06cf`, 2025-12-08) | 27 字节一致 | 27 字节一致 | 通过 | 通过 |
| Node.js | v25.3.0 | 27 字节一致 | 27 字节一致 | 通过 | 通过 |
| C | Apple clang 21.0.0 (`clang-2100.1.1.101`) | 27 字节一致 | 27 字节一致 | 通过 | 通过 |
| C++ | Apple clang 21.0.0 (`clang-2100.1.1.101`) | 27 字节一致 | 27 字节一致 | 通过 | 通过 |
| Go | go1.26.6 darwin/arm64 | 27 字节一致 | 27 字节一致 | 通过 | 通过 |
| shell | `/bin/sh`，Bash 3.2.57(1)-release | 27 字节一致 | 27 字节一致 | 通过 | 通过 |

Python 来自 `/Users/a1234/.pyenv/versions/3.14.1/bin/python3`；Rust 来自 `/Users/a1234/.cargo/bin/rustc`；Node.js 和 Go 来自 `/opt/homebrew/bin`；C / C++ 分别使用 `/usr/bin/cc` 和 `/usr/bin/c++`。未安装额外运行时或编译器。

## 验证方法与边界

每个程序先分别向 stdout / stderr 写入以下已知字节，并在需要时显式 flush：

```python
stdout_first = b'OUT\x00\xffno-newline:part1'
stderr_first = b'ERR\x00\xffno-newline:part1'
tail = b':part2'
```

两份数据均没有换行，含 NUL 和无效 UTF-8 字节 `0xff`。源程序随后等待一个临时门控文件，验收脚本确认两个原始输出文件已分别等于首段字节、且源插件仍在运行后，才创建门控文件，让程序写入尾段并退出。采集的 `chunk_bytes=3`，强制经过多个读取块；最终逐字节比较两个输出文件，验证内容完整和 stdout / stderr 分流。测试不假设两条流之间有全局顺序。

最终每条流为 27 字节，SHA-256 为：

| 流 | SHA-256 |
| --- | --- |
| stdout | `789597dc69ea74dc02ef692a7df179ad53b705b6199703524a646a686f744ceb` |
| stderr | `c5ef174c3d82719b795fc4a2cd87d0f726f4b00748bd9f1a98800ce55ff539fe` |

这证明表中具体程序与版本的实际输入链路可用。程序自身仍须按其运行时的缓冲规则写出或 flush；测试没有声称采集端能读取尚在语言运行时缓冲区内的字节。本项不覆盖每种语言的全部日志框架、PTY 行为、串口硬件或性能上限。Linux / Windows 的实际执行结果须由对应平台的验收产物确认，不能由本机结果推断；脚本已提供 Windows 二进制管道 fixture 和 PowerShell 分支。

## 复跑与 CI 入口

在仓库根目录运行：

```sh
cargo build --workspace
python3 examples/io/languages.py --json artifacts/input-languages.json
```

Windows 使用实际的 Python 命令（通常为 `python`）。也可通过 `--bin-dir target/release` 指向已构建的 release 二进制。脚本只使用已有工具，源文件、编译输出、运行目录和 Go 构建缓存置于独立临时目录。测试结束会关闭其启动的进程并清理临时文件。

Python 和 Rust 是必需项，缺失即失败；Node.js、C、C++、Go、shell 缺失会记录 `skipped` 和原因。已找到的工具若编译或执行失败，则记录 `failed`，不会当作跳过。任何失败使脚本返回非零退出码。标准输出和可选 JSON 文件包含逐语言版本、状态、字节校验、被测二进制哈希及执行时间，便于 CI 留存。

## 文件替换覆盖（独立 I/O 验收）

语言 stdout/stderr 覆盖不代表所有语言的文件轮转 API 都可用。Windows 旧验收实际遇到 Python 3.12 `Path.replace` 返回 `WinError 5`：它使用的 `MoveFileExW` 不允许覆盖仍打开的目标，即使读取方已共享 DELETE。核对 `same-file 1.0.6 → winapi-util 0.1.11` 后，读取及身份句柄都保留 Rust 默认的 READ / WRITE / DELETE 共享，并非采集插件漏设共享位。[Python 官方说明](https://bugs.python.org/issue46003)、[Rust 共享模式](https://doc.rust-lang.org/std/os/windows/fs/trait.OpenOptionsExt.html#tymethod.share_mode)。

| 平台与替换入口 | 本次覆盖 |
| --- | --- |
| macOS：Python `Path.replace` | 真实跟随进程中的完整替换字节和段变更验收 |
| Windows：Python 3.12 `Path.replace` / 旧 `MoveFileExW` | 已实测拒绝替换打开的目标；保留为 API 限制，不计通过 |
| Windows：临时 Rust `std::fs::rename` 程序 | a0be668的[三平台CI](https://github.com/TXyy2023/log_print/actions/runs/34150338258)实际通过；真正替换打开的目标，核对完整字节及 `reason=replaced` / segment |

Windows 夹具所用现代 Rust 会在需要时调用 `FileRenameInfoEx` 的 `REPLACE_IF_EXISTS | POSIX_SEMANTICS`；旧文件句柄保持有效，路径指向新文件。这是实际替换，不是截断、停止读取或跳过。测试临时生成源码并使用现有 `rustc` 编译，结束后清理；该 I/O 测试在 Windows 上需要 Rust 编译器。[Rust 实现](https://github.com/rust-lang/rust/blob/1.98.0/library/std/src/sys/fs/windows.rs)、[微软语义说明](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/4217551b-d2c0-42cb-9dc1-69a716cf6d0c)。

## 本次被测构建

执行开始时间为 `2026-09-07T17:16:49Z`（北京时间 2026-09-08 01:16:49），使用工作区 `target/debug` 的 1.0.0 二进制。源码当时尚未提交，旧初始化提交不能代表本次被测构建；以下哈希精确标识实际执行文件。本项为功能覆盖验收，未修改或重测性能报告。

| 二进制 | SHA-256 |
| --- | --- |
| input-program | `4e16bf287d4ce0b8d72604f24ba523aed3265d19f0a0ec3a618c7c76ea29a1e0` |
| log-print-core | `fcc765e6ca7cb82939463aceaf461557b2ec6f33acb62319911482cd918cf38b` |
| output-raw | `30fde3d3b599c82472f90790034db71b6c81af0af8fda19f7e833b31a071bb6a` |
