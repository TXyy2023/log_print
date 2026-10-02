<span id="input-program-启动程序与-tmux-接入"></span>

# input-program: processes and tmux

[Guide](../guides/program.md) · [Common configuration](../reference/configuration.md)

Each instance writes one Core-assigned stream. Both modes use a startup snapshot: restart the main program after configuration changes; restarting only the plugin does not reload them.

<span id="启动模式"></span>

## Spawn mode

```json
{"mode":"spawn","command":"python3","args":["-u","app.py"],"chunk_bytes":4096,"shutdown_ms":2000}
```

| Field | Default and range |
|---|---|
| `mode` | `spawn`; alternatively `tmux` |
| `command` / `args` | Required executable in spawn mode; arguments default to an empty array; no implicit shell |
| `cwd` / `env` | Inherit working directory and environment by default; optional overrides |
| `chunk_bytes` | 4096; 1–65536; encoded UDP limits also apply |
| `shutdown_ms` | 2000; 10–10000 ms to wait for source-process cleanup after termination |

stdout/stderr use pipes, without a PTY; stdin is closed. Records preserve raw bytes without waiting for newlines. `channel` distinguishes `stdout` and `stderr`, each with its own increasing `source_seq`. Core `seq` reflects receive order and cannot reconstruct production order across pipes. Source buffering is controlled by the source program's own flush/unbuffered settings.

The target does not inherit plugin connection variables prefixed `LOG_PRINT_*`. A nonzero target exit or detected Core connection error reports failure. UDP lacks TCP EOF notification; the supervisor handles Core crashes. Manual stop terminates the plugin-created Unix process group or Windows Job and attempts to reap descendants. Unix daemons that actively leave the group are outside that cleanup scope. If descendants keep output pipes open, collection waits for closure or manual stop.

<span id="tmux-模式"></span>

## tmux mode

```json
{"mode":"tmux","tmux_target":"%3","chunk_bytes":4096}
```

Requires Unix and tmux. `tmux_target` is required; prefer an explicit pane ID. Optional `tmux_socket` selects the server socket, as with `tmux -S`; otherwise the current tmux environment or default server is used. This mode rejects `command/args/cwd/env` and does not restart the target.

Only new output is captured, not earlier screen history. Records contain terminal bytes, possibly control codes; `channel` is `terminal`, without recoverable stdout/stderr separation. Existing `pipe-pane` connections are refused. Synchronized tmux conditional commands coordinate checking and installation so another collector's pipe is not closed in a race.

A dedicated helper transfers bytes through a private Unix socket. Stop disconnects only its own connection and exits the helper; the target continues. Cleanup does not close a pipe subsequently replaced by someone else. Arbitrary PID attachment, ordinary TTY attachment and complete screen-history recovery are not provided.

In either mode, EOF is not downstream completion. Rolling buffers can overwrite records and UDP send success is not Core receipt. Lossless completeness is not guaranteed. The old `stdout_stream/stderr_stream` configuration has been removed.
