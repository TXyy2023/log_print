<span id="安装与构建"></span>

# Install and build

This manual follows the current source tree. A workspace build produces the main CLI, Core and seven official plugins.

<span id="准备环境"></span>

## Prerequisites

Install Git, Rust 1.92 or a newer compatible stable toolchain, and your platform's Rust compiler/linker prerequisites. The quick start also uses Python 3 to generate sample logs.

```sh
rustc --version
cargo --version
python3 --version
```

On Windows, the Python command is usually `python`. Python is used by these examples and tests; it is not a runtime dependency of the application. The WebUI assets are checked in and embedded in the Rust binary. Node.js and npm are needed only to rebuild the frontend or documentation, or to run the full validation suite; CI uses Node.js 24.

<span id="获取并构建"></span>

## Clone and build

```sh
git clone https://github.com/TXyy2023/log_print.git
cd log_print
cargo build --release --locked --workspace
./target/release/log-print --version
```

For an existing checkout, run the build command there. The workspace version is currently `0.1.2`, but `main` includes CLI, WebUI and TUI changes added after the original `ver-0.1.2` tag. A version string alone does not establish identical interfaces: use the manual and binaries from the same commit. Local uncommitted changes are not included in a fresh clone. `--locked` uses the committed dependency lockfile; the first build downloads dependencies and is not an offline installation procedure.

The executables are in `target/release/`:

- `log-print`: start, manage and read an instance.
- `log-print-core`: Core, launched by the main program.
- `input-program`, `input-file`: input plugins.
- `output-raw`, `output-transform`, `output-file`, `output-webui`, `output-tui`: display, transformation and archive plugins.

The CLI locates the configured plugins. Run the examples from the repository root and keep these executables together in the build directory; copying only `log-print` does not install the other components.

<span id="命令约定"></span>

## Command conventions

Examples run from the repository root with a release build. On Windows, replace `./target/release/log-print` with `target\release\log-print.exe`, and `python3` with your available `python` command.

Relative data paths in plugin configuration are resolved from the runtime working directory, not the JSON file's directory. Tutorials use separate `--state` paths; every command for an instance must use the path it was started with.

Next: [Quick start](quickstart.md).
