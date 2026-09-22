# 检查与安装 log-print

Skill 可单独安装，但采集需要 CLI、Core 和使用到的插件。源代码与维护入口：<https://github.com/TXyy2023/log_print>。

## 先检查

macOS / Linux，在用户提供的源码根目录或当前工作目录中：

```sh
command -v log-print
test -x ./target/release/log-print && ./target/release/log-print --version
```

第一条未找到或第二条路径不存在不代表另一位置也不可用。选择实际找到的程序后设定路径，例如源码构建产物：

```sh
LOG_PRINT_BIN="$(pwd)/target/release/log-print"
"$LOG_PRINT_BIN" --version
"$LOG_PRINT_BIN" --help
"$LOG_PRINT_BIN" read --help
```

若选择 PATH 中的程序，改用 `LOG_PRINT_BIN="$(command -v log-print)"`。检查同级目录中的 `log-print-core` 和配置使用的插件；例如文件采集需要 `input-file`，归档需要 `output-file`。同级缺失时再核对 PATH。不对 Core/插件执行 `--version` 或 `--help`。

Windows PowerShell：

```powershell
Get-Command log-print -ErrorAction SilentlyContinue
Test-Path .\target\release\log-print.exe
$LogPrintBin = (Resolve-Path .\target\release\log-print.exe).Path
& $LogPrintBin --version
& $LogPrintBin --help
& $LogPrintBin read --help
```

若选择 PATH 中的程序，改用 `$LogPrintBin = (Get-Command log-print).Source`；Core 和插件同样带 `.exe`。

## 缺少时从源码构建

需要 Git、Rust **1.92 或更新版**及当前平台的编译工具链。先执行 `git --version`、`rustc --version`、`cargo --version`；缺少 Rust 时使用[官方安装入口](https://www.rust-lang.org/tools/install)。tmux 模式额外需要 Unix 与已安装的 tmux，普通文件/程序采集不需要 tmux。

下面把公开的 0.1.2 版本克隆到一个**尚不存在**的新目录。已有 checkout 时先检查分支和未提交改动，不覆盖用户的工作目录，也不为了安装执行 reset/clean。

```sh
git clone --depth 1 --branch ver-0.1.2 https://github.com/TXyy2023/log_print.git log-print-0.1.2
cd log-print-0.1.2
cargo build --release --workspace --locked
```

这条工作区构建会把以下程序放在同一 `target/release` 中：

```text
log-print
log-print-core
input-file
input-program
output-raw
output-file
output-transform
```

Windows 文件名带 `.exe`。只构建或安装 `app-log-print` 不会带齐 Core 和独立插件；不要使用未经验证的同名包作为替代。默认推荐直接使用本次仓库的 `target/release/log-print`；也可仅为当前终端把**整个目录**加入 PATH：

```sh
export PATH="$(pwd)/target/release:$PATH"
log-print --version
log-print --help
```

PowerShell 对应：

```powershell
$env:Path = "$(Join-Path (Get-Location) 'target\release');$env:Path"
log-print --version
log-print --help
```

这些命令不修改用户的 shell 配置。构建成功后执行 [workflows.md](workflows.md) 中与当前任务相符的有限采集，验证启动、读取和归属内停止；版本输出本身不证明整条链路可用。
