# 排查常见问题

| 现象 | 检查与处理 |
| --- | --- |
| 配置提示 unknown field / role missing | 对照 [配置参考](reference/configuration.md) 迁移；旧 save 和多流 Input 不再接受 |
| 找不到流 | 先 `streams` 取得当前 UUID；Core 重启后身份变化 |
| 后启动 Output 缺少开头 | 有界缓冲已覆盖；Core 无磁盘历史可补取 |
| UDP 已发送但没看到数据 | 本地发送不保证收到；检查注册、报文编码大小及插件错误 |
| 修改 config 后没变化 | 运行期使用启动快照，需停止并重新启动主程序 |
| Output 启动失败 | 检查 reads、UUID、输出路径和 `status` / stderr；目标存在不会覆盖 |
| tmux 接入被拒绝 | 检查窗格是否已有 pipe-pane；不要抢占其采集管道 |
| 插件停止 forced=true | 收尾未及时成功；检查错误及实际保存状态，不能视为完整保存 |

`start` 返回 stdout/stderr 日志路径，`status` 返回进程与业务状态。状态文件含管理凭据，勿公开。不要删除正在运行实例的状态文件绕过冲突。
