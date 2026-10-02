<span id="静态文件快速导入"></span>

# Import a static file

Version 0.1.2 consolidates file replay into `input-file`; the separate `input-replay` executable is no longer built. Use this plugin declaration:

```json
{"id":"import","role":"input","bin":"input-file",
 "streams":[{"id":"imported","description":"Static file"}],
 "config":{"path":"source.log","mode":"static"}}
```

Static mode reads from byte zero as quickly as possible. The plugin exits at EOF, while its Core stream and retained buffer remain. It does not reproduce historical timing, offer speed multipliers, parse timestamps or replay SQLite archives.

Finishing the source read, acceptance by Core, and completion by Outputs are separate events. Core does not wait for Outputs; a fast import can overwrite records before downstream consumers read them. Check each Output's results rather than interpreting a successful source exit as a complete batch import.

[File plugin reference](../plugins/input-file.md) · [Integrity limits](recovery.md)
