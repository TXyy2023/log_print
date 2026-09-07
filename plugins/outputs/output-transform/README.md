# output-transform

消费父流，转换后发布到同一个 Core 的独立派生流。

所有插件由 `log-print` 注入 `LOG_PRINT_CORE/PLUGIN/TOKEN/CONFIG`，通过 Rust SDK 的本地 TCP IPC 连接 Core；stdout 不承载协议，诊断写 stderr。下列 JSON 放在插件声明的 `config`。流须同时出现在声明的 `streams`/`reads`，保存由 Core 按声明执行。默认读到即推，不等待换行；每次发布保留原 key 等待明确保存恢复，其他错误退出且报告。

所有插件接受 `shutdown`、`config.get`；下表列出的动态字段接受 `config.patch`，只改本次进程、不写配置文件。其余字段重启插件生效。

```json
{"streams":["program.out"],"output_stream":"clean","input_encoding":"utf-8","output_encoding":"utf-8","split_lines":true,"keep_newline":true,"prefix":"","suffix":"","delete":["DEBUG: "],"replace":[{"pattern":"temp=","with":"temperature="}]}
```

插件声明须同时包括 `reads:["program.out"]` 与 `streams:[{"id":"clean","parents":["program.out"]}]`。多父场景 streams 与 parents 集合必须一致；逐父保持解码/分行状态。发布携带全部父的最近已消费 seq，尚未见齐父流时有界暂存；超限报错，禁止伪造父进度。父 gap/epoch 变化停止，避免拼接跨缺口的字符或行。重启生成新 run key，会重新派生请求范围，不宣称跨重启 exactly-once。

| 字段 | 默认/范围 | 生效 |
|---|---|---|
| streams / output_stream / from | 非空 / derived / 1 | 重启 |
| input_encoding | auto；也可 raw 或 encoding_rs 标签 | 重启 |
| output_encoding | utf-8；raw 或显式编码标签 | 重启 |
| radix | none / hex_encode,hex_decode,bin_encode,bin_decode,dec_encode,dec_decode | 重启 |
| split_lines / keep_newline / flush_partial | false / true / true | 重启 |
| max_pending_bytes | 65536，4..1048576；单行及首次等父流限制 | 重启 |
| prefix / suffix | 空 | 动态 |
| delete / replace | 空数组；最多 64 规则，pattern ≤4096 字节 | 动态 |

处理顺序：进制 decode → 字符解码 → 跨块按 LF 分行 → delete/replace → prefix/suffix → 目标编码 → 进制 encode。进制是字节的数值表示，和字符编码不同。数值 token 用空白分隔，支持十六进制 0x、二进制 0b 前缀；跨块 token 保留到空白或 shutdown，不把一字节拆包当 token 结束。encode 使用小写 hex、8 位 binary 或十进制并加空格，三组可往返任意字节。数值用途推荐 input_encoding=raw（此时忽略字符目标编码）。

手动编码使用 [encoding_rs 流式 Decoder](https://docs.rs/encoding_rs/latest/encoding_rs/)，支持跨块多字节字符，非法字节或不可表示目标直接报错；UTF-16 输出显式处理，UTF-32/UTF-7 不在支持范围。auto 验证 UTF-8 并用 [chardetng 0.1.17](https://docs.rs/chardetng/0.1.17/chardetng/struct.EncodingDetector.html) 报告最可能的旧编码。检测器 assess 布尔值不是置信概率，所以不凭它自动转写旧编码；不确定字节保留原样并跳过文本编辑，用户通过明确 input_encoding 转换。按字节分行的 auto/raw 不用于可靠解析 UTF-16，应手动指定编码。

删除/替换采用 [Rust regex](https://docs.rs/regex/latest/regex/)，替换支持 `$1` 等捕获。规则动态更新会用于下一次完成的单位，包括已缓存部分行。单次编辑输出上限为 max_pending_bytes 的 4 倍，再分成 ≤65536 字节发布；数值格式展开也有固定倍数上限。正常 shutdown 会尝试 flush 尾字符/末行，2 秒超时则报告未确认；Core 没有源 EOF 事件，因此无换行的最后部分行可等待 shutdown 才输出。任何失败均保留 Core 原始父流不变。

选型：encoding_rs/chardetng/regex 使用成熟 Rust 实现，减少自写编码表和回溯正则风险；许可证分别为 encoding_rs `(Apache-2.0 OR MIT) AND BSD-3-Clause`、chardetng `Apache-2.0 OR MIT`、regex `MIT OR Apache-2.0`。版本以 Cargo.lock 为准。自动旧编码保守保留是实施默认，不能宣称任意二进制均可自动准确识别。

`from:0` 表示连接订阅时仅接收新记录，`from:1` 请求历史起点；默认 1。`config.get` 返回填齐默认后的有效值、每字段来源及动态字段。
