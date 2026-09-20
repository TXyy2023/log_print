# 恢复与完整性

本页讨论 `output-file` 的已有归档续写。开始前先区分源历史是否可重读，以及归档是否已有持久游标。

## 恢复已有归档

1. 确认旧归档插件已停止，未继续持有输出目标。若 shutdown 超时，先检查进程和状态，不立即启动第二个写入者。
2. 保留目标文件、索引、checkpoint、数据库及所需伴随文件；保留 Core 保存目录和原有流身份。
3. 在原配置中将归档插件的 `mode` 从 `create` 改为 `resume`。保持流集合、文件格式和所有目标路径一致。
4. 用修改后的配置启动实例；若 Core 仍在运行，确保启动配置已更新或通过正确的实例重启流程加载文件，不能只改磁盘 JSON 后假定运行中的插件声明随之变化。
5. 查看 `archive status.get`，核对身份、缺口、错误和各目标进度。恢复起点由持久游标决定，`from` 不用于覆盖恢复位置。

恢复会校验已确认的内容与状态，只有验证通过才处理未确认尾部。文件变短、已确认内容变化、身份或状态不匹配时会停止，不会自动重建为一个看似完整的新归档。

## 续开双目标示例

完成 [双目标示例](archive.md)并正常停止后，将原配置另存为仓库根目录的 `both-resume.json`，只改两处：

- `archive` 插件的 `config.mode` 改为 `resume`。
- `replay` 插件声明的 `autostart` 设为 `false`，保留其流声明。

然后从同一仓库根目录执行：

```sh
./target/release/log-print --state capture/both-state.json start --config both-resume.json
./target/release/log-print --state capture/both-state.json plugin call archive status.get
```

此时 Core 重开原保存历史，归档校验已有进度；不会再次运行回放输入。源历史已归档完整时，文件不应新增重复字节。确认状态后：

```sh
./target/release/log-print --state capture/both-state.json plugin call archive shutdown
./target/release/log-print --state capture/both-state.json stop
```

归档恢复不等于输入源断点续采。重新运行 `input-replay` 会重新发布文件，重新运行程序输入会重新启动源程序；这些新发布记录可以合法进入后续归档。不要把重新执行源任务产生的重复内容误认为归档恢复重复写入。

## Core 也要重启时

要补齐归档插件没收到的记录，Core 必须保留对应历史和 epoch。启动一个全新的、未保存的 Core 不能替代旧历史；epoch 不匹配会被拒绝。

Core 保存与归档保存是两份不同的数据。不要直接编辑 Core 内部数据库，也不要把归档数据库放进 Core 保存目录来代替它。

## 双目标中的领先进度

文件与 SQLite 分别提交，没有跨两者的原子事务。故障时一个目标可能领先；共同游标取较小进度，恢复时领先目标验证重复记录一致后跳过。

任一目标失败会停止归档，不会自动降级为只写另一个目标。

## 正常停止的顺序

先停止或等待输入完成；有转换时先完成转换收尾；确认归档共同进度追上源流最终位置，再 shutdown 归档，最后停止实例。

归档 shutdown 排空它已接收的数据并提交，不代表整条流水线所有数据都已到达它。Core 控制等待为 10 秒，超时意味着结果未知，归档可能仍在收尾。

## 如何理解保证

- 缺口默认持久记录后停止；允许继续也会保留不完整标记。
- 断连不自动重连，不把未知损失量报告成零。
- 同步和提交使用系统持久化接口，真实断电行为仍受文件系统及硬件影响。
- 进程崩溃、错误注入与真实坏盘、断电验收不是同一种证据。

存储校验、队列和提交细节见 [output-file 参考](../plugins/output-file.md)。
