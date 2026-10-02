<span id="install-and-build"></span>

# 安装与构建

本手册随当前源码维护，使用源码构建方式。构建结果包括主程序、Core 和七个官方插件。

<span id="prerequisites"></span>

## 准备环境

需要 Git、Rust 1.92 或更新的兼容稳定工具链，以及所在系统的 Rust 编译和链接环境。快速开始还使用 Python 3 写入教学日志。

```sh
rustc --version
cargo --version
python3 --version
```

Windows 下 Python 命令通常为 `python`。Python 只用于本手册中的输入生成、部分示例和测试，不是主程序的运行依赖。WebUI 构建产物已提交并嵌入 Rust 二进制；仅重新构建前端、文档站及运行全套验收需要 Node.js/npm，CI 使用 Node.js 24。

<span id="clone-and-build"></span>

## 获取并构建

```sh
git clone https://github.com/TXyy2023/log_print.git
cd log_print
cargo build --release --locked --workspace
./target/release/log-print --version
```

已有工作区时直接在该工作区执行构建命令。当前 workspace 版本号为 `0.1.2`，CLI 接口已在本地源码中调整；旧 `ver-0.1.2` 标签不包含这些调整，不能仅根据版本号判断接口一致。阅读源码手册时应使用同一份源码构建；本地未提交改动不会自动出现在新克隆的仓库中。`--locked` 要求按仓库锁文件解析依赖；首次构建需要获取依赖，本文不将其作为离线安装流程。

构建结果位于 `target/release/`：

- `log-print`：启动、管理和读取实例。
- `log-print-core`：由主程序启动的 Core。
- `input-program`、`input-file`：输入插件。
- `output-raw`、`output-transform`、`output-file`、`output-webui`、`output-tui`：输出和转换插件。

主程序会查找配置声明的插件。按照本手册在仓库根目录运行，并保留上述可执行文件在构建目录中，不要只拷贝主程序后假定其余组件已安装。

<span id="command-conventions"></span>

## 命令约定

后续命令默认在仓库根目录运行，使用 release 构建。Windows 将 `./target/release/log-print` 替换为 `target\release\log-print.exe`，将 `python3` 替换为可用的 `python`。

配置中的相对文件路径以运行时工作目录为基准，不以 JSON 文件所在目录为基准。不同教程使用不同 `--state` 路径；操作某个实例时，始终使用它启动时的路径。

下一步：[快速开始](quickstart.md)。
