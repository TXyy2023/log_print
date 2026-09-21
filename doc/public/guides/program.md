# 采集程序输出

`input-program` 可通过我们的 CLI 启动目标，或接入已有 tmux 窗格。一个实例对应一条 Core 流。

## 通过 CLI 启动目标

将下列完整配置保存为 `program.json`。示例生成两段测试文本；自己的程序替换 `command` 和 `args` 即可，Windows 可将 `python3` 改为 `python`。

```json
{
  "plugins": [{
    "id": "program", "role": "input", "bin": "input-program",
    "streams": [{"id": "program-data", "description": "程序 stdout 与 stderr"}],
    "config": {
      "mode": "spawn", "command": "python3",
      "args": ["-u", "-c", "import sys; print('hello stdout'); print('hello stderr', file=sys.stderr)"]
    }
  }]
}
```

```sh
./target/release/log-print --state .log-print/program.json start --config program.json
./target/release/log-print --state .log-print/program.json streams
```

用返回的实际流 UUID 替换 `STREAM_ID`：

```sh
./target/release/log-print --state .log-print/program.json read STREAM_ID
./target/release/log-print --state .log-print/program.json status
./target/release/log-print --state .log-print/program.json stop
```

记录的 `channel` 区分 stdout/stderr，两个管道各自保持字节顺序，不承诺彼此的精确产生顺序。使用 `--raw` 只输出合流后的字节，会丢掉显示中的通道标签。`read` 是当前缓冲有限快照，持续处理请使用 Output 插件。

`command` 不隐式调用 shell，管道与重定向不能直接写成整串命令。可用 `cwd` 和 `env` 调整目标环境；stdin 关闭，不分配交互终端。Python `-u` 控制源程序缓冲，其他程序使用其自身的 flush 方式。非零退出报告失败。源结束不会停止 Core；手动停止仍运行的插件会终止它创建的进程组/Job 及其中后代。

## 接入已有 tmux 程序

在 Unix 上先用 `tmux list-panes -a` 找到目标，建议使用明确的 `%窗格ID`。保留上面的插件声明，修改业务配置：

```json
{"mode":"tmux","tmux_target":"%3"}
```

目标已经运行，不需重启。接入只读取之后产生的新输出，不导入之前的屏幕历史；终端控制码会保留，`channel` 为 `terminal`。目标已有 `pipe-pane` 时拒绝，不抢占其他采集器。若使用独立 tmux 服务，可增加 `tmux_socket` 指定其 socket 路径。

```sh
./target/release/log-print --state .log-print/program.json plugin stop program
```

在 tmux 模式下，这个停止命令只解除本插件采集，目标程序继续运行。启动模式的停止会终止它创建的程序，两个模式的停止语义不同。

所有配置随主程序启动固定，修改文件后需重启主程序才生效。有限滚动缓冲可能覆盖未读内容，源 EOF 不代表 Output 已处理完成；UDP 也不保证交付。[完整参数与边界](../plugins/input-program.md)。
