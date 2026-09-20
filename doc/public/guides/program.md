# 采集程序输出

`input-program` 启动一个程序，分别采集它的 stdout 和 stderr。Python、Node.js、Java 或编译后的程序使用同一种接入方式；相应运行时需要自行安装。

## 跑通一个完整例子

将以下配置保存为仓库根目录的 `program.json`。示例 Python 程序写两段教学文本，停留 1 秒后退出，便于主程序完成启动握手。

```json
{
  "plugins": [{
    "id": "program", "bin": "input-program",
    "streams": [{"id": "program.out"}, {"id": "program.err"}],
    "config": {
      "command": "python3",
      "args": ["-u", "-c", "import sys,time; print('hello stdout'); print('hello stderr', file=sys.stderr); time.sleep(1)"],
      "stdout_stream": "program.out", "stderr_stream": "program.err"
    }
  }]
}
```

Windows 将配置中的 `python3` 改为 `python`。

```sh
./target/release/log-print --state .log-print/program.json start --config program.json
./target/release/log-print --state .log-print/program.json read program.out --raw --wait-ms 1000
./target/release/log-print --state .log-print/program.json read program.err --raw --wait-ms 1000
./target/release/log-print --state .log-print/program.json stop
```

两次读取分别应得到 `hello stdout` 和 `hello stderr`。源程序结束后，实例仍需显式停止。

## 接入自己的程序

修改 `command` 和 `args`；`command` 是可执行文件，`args` 是逐项传入的参数数组。插件不隐式调用 shell，不能直接把管道、重定向或整条带空格命令写进 `command`。

可用 `cwd` 指定源程序工作目录，用 `env` 覆盖所需环境变量。源程序不获得交互 stdin，因此不适合需要终端交互输入的程序。

## 输出顺序和缓冲

stdout、stderr 各自保持字节顺序，但不承诺两者之间的精确先后关系。源程序自己的输出缓冲也会影响可见时间；Python 示例中的 `-u` 用于禁用缓冲，其他语言应使用对应的 flush 机制。

非零退出会作为失败报告。正常关闭会处理插件创建的进程组或 Windows Job；主动脱离 Unix 进程组的守护进程不在这一回收范围内。

更多字段见 [input-program 参考](../plugins/input-program.md)。
