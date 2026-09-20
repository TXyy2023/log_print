# 转换日志

`output-transform` 读取原始流，处理文本或字节表示，再发布一个独立派生流。原始内容仍可读取。

## 将 temp= 替换为 temperature=

将以下完整配置保存为仓库根目录的 `transform.json`。启动前确保 `example.log` 存在。

```json
{
  "plugins": [
    {
      "id": "file", "bin": "input-file",
      "streams": [{"id": "logs"}],
      "config": {"path": "example.log", "stream": "logs", "from_start": false}
    },
    {
      "id": "transform", "bin": "output-transform", "reads": ["logs"],
      "streams": [{"id": "clean", "parents": ["logs"]}],
      "config": {
        "streams": ["logs"], "output_stream": "clean",
        "input_encoding": "utf-8", "output_encoding": "utf-8",
        "split_lines": true, "keep_newline": true,
        "replace": [{"pattern": "temp=", "with": "temperature="}]
      }
    }
  ]
}
```

```sh
./target/release/log-print --state .log-print/transform.json start --config transform.json
python3 -c "open('example.log','ab').write(b'temp=23.5\n')"
./target/release/log-print --state .log-print/transform.json read clean --raw --wait-ms 1000
./target/release/log-print --state .log-print/transform.json stop
```

派生流预期输出 `temperature=23.5` 并保留换行；读取 `logs` 仍得到原始 `temp=23.5`。

## 选择处理方式

| 需求 | 配置方向 |
| --- | --- |
| 按完整行替换或加前后缀 | `split_lines: true`，配置 `replace` / `delete` / `prefix` / `suffix` |
| 已知旧编码转 UTF-8 | 明确指定 `input_encoding`，使用 `output_encoding: utf-8` |
| 任意字节转十六进制表示 | `input_encoding: raw`，`radix: hex_encode` |
| 从数值表示还原字节 | 选择 `hex_decode` / `bin_decode` / `dec_decode` |

删除和替换使用 Rust regex 规则。自动编码模式在不确定时保留原字节并跳过文本编辑，不保证准确识别任意编码。

## 尾行和重启

启用分行后，没有换行的末尾部分可能等到转换插件 shutdown 才输出，因为 Core 没有源 EOF 事件。需要完整导出时，先让转换插件完成收尾，再确认下游进度。

父流缺口或 epoch 变化会停止转换；重启可能重新派生请求范围。处理顺序、编码范围、动态参数和内存边界见 [output-transform 参考](../plugins/output-transform.md)。
