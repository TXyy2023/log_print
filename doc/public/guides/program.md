<span id="采集程序输出"></span>

# Capture program output

`input-program` launches a target process or attaches to an existing tmux pane. Each plugin instance produces one Core stream.

<span id="通过-cli-启动目标"></span>

## Launch a process

Save this complete configuration as `program.json`. It produces two sample messages; substitute your program's `command` and `args`. On Windows, use `python` if appropriate.

```json
{
  "plugins": [{
    "id": "program", "role": "input", "bin": "input-program",
    "streams": [{"id": "program-data", "description": "Program stdout and stderr"}],
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

Replace `STREAM_ID` with the returned UUID:

```sh
./target/release/log-print --state .log-print/program.json read STREAM_ID
./target/release/log-print --state .log-print/program.json status
./target/release/log-print --state .log-print/program.json stop
```

The record's `channel` distinguishes stdout from stderr. Byte order is preserved within each pipe; exact production order across the two pipes is not guaranteed. `--raw` emits only interleaved payload bytes, without channel labels. `read` is a bounded snapshot; use an Output for continuous processing.

`command` does not implicitly invoke a shell, so shell pipes and redirections cannot be supplied as a single command string. Set `cwd` and `env` to customize the environment. Stdin is closed and no interactive terminal is allocated. Python's `-u` controls source buffering; other programs need their own flush settings. A nonzero target exit is reported as failure. Source exit does not stop Core. Manually stopping a running spawn plugin terminates its process group/Job and attempts to reap its descendants.

<span id="接入已有-tmux-程序"></span>

## Attach to an existing tmux pane

On Unix, use `tmux list-panes -a` to find the target; prefer an explicit `%paneID`. Keep the plugin declaration above and replace its business configuration with:

```json
{"mode":"tmux","tmux_target":"%3"}
```

The target keeps running. Only new output is captured; earlier screen history is not imported. Terminal control codes remain in the bytes and `channel` is `terminal`. If the pane already has a `pipe-pane`, attachment is refused rather than taking it over. Use `tmux_socket` for a separate server's socket path.

```sh
./target/release/log-print --state .log-print/program.json plugin stop program
```

In tmux mode, this detaches only this collector and leaves the target running. In spawn mode, stopping terminates the program it created.

Configuration is a startup snapshot; restart the main program after changing it. Bounded buffers can overwrite unread data, source EOF does not mean downstream completion, and UDP does not guarantee delivery. See [input-program](../plugins/input-program.md).
