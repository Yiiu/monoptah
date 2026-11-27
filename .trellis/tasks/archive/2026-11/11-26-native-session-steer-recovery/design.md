# 会话追问与停止恢复设计

交付确认和会话恢复以 Orca 为主要参考。排队、追问与停止后发送参考 T3 Code 和 VS Code。T3 Code 的 Force Stop 还提供了按目标 run 校验恢复操作的先例。具体实现遵循本地 provider 的协议。[Orca](research/orca-follow-up-recovery.md) · [T3 Code](research/t3-code-session-controls.md) · [同类工具与 Codex 协议](research/session-controls-comparison.md)

## 路径

本地会话从 `conversation/follow_up.rs` 的 `message_queue` 接收点击，持久化待发项后由 `pump_follow_ups` 通过 `Control::Steer` 交给 runner。Codex 发送 `turn/steer` 后以 JSON-RPC 请求 ID 匹配回复，runtime 落盘接收凭据再通知界面移出队列。停止由 Conversation 设置取消标识，再由 runner 发送 `turn/interrupt` 并汇报终态。

| 位置 | 已知条件 |
| --- | --- |
| `crates/monocode-gpui/src/conversation/follow_up.rs:320` | 本地派发受会话加载、队列持久化、暂停、是否正在停止、provider 就绪和是否已有发送请求等条件限制。明确选择的追问可绕过暂停门槛。 |
| `crates/monocode-core/src/provider/follow_up.rs:123` | 同一个待发项的请求 ID 与回执配对。Codex 未纳入现有的追问确认超时处理。 |
| `crates/monocode-core/src/provider.rs:1902` | 正在中断时继续读取确认或对应的终态。Codex 的确认等待没有独立期限。 |
| `crates/monocode-gpui/src/conversation.rs:1922` | 收到最终更新才收尾并重新派发队列。 |
| `crates/monocode-core/src/store/shared.rs:2786` | 队列与对话历史分别有并发校验。已有跨窗口保护和数据库兼容性需要纳入测试。 |

## 设计约束

1. 先用本地脱敏事件复现真实阻塞点。将队列消息 ID、回合 ID、请求 ID 和阶段串起来。日志记录阶段、耗时、就绪及阻塞原因，严格排除提示词、附件内容和凭据。分别记录 Steer 被点击、队列保存、成功写入 provider、收到确认、结束回合。
2. 表示派发前、等待确认、确认成功、确认拒绝和结果未知。现有队列行提供反馈，持久化队列和当前回合状态共同决定允许的操作。对明确点选的普通追问，若当前轮无法派发，记录具体原因并保持该条待发。
3. Codex Steer 的确认等待阈值为 15 秒。超过阈值将请求显示为结果未知，同时保留旧回合连接上迟到的确认处理。确认表示代理接收，模型实际处理由后续回合事件体现。确认收到时按现有接收凭据只落一次。单纯等待超时不得触发自动重发。结束或重连后根据回执及可核实的已提交输入判断是否可安全继续入队。对仍不明确的消息保留并提示用户。
   已确认 Codex CLI 0.172.0 支持可选的 `clientUserMessageId`。支持时使用稳定的待发项 ID，配合 `userMessage.clientId` 核对原消息，JSON-RPC 请求 ID 仍用于关联每一次派发。旧版本或缺少回显时保留现有回执路径，内容相同但身份不明的消息保留为未确认。
4. Stop 走现有中断路径。发送 Codex 中断并等待回执的预算为 3 秒，收到回执后等待对应终态的预算为 5 秒。通过现有进程隔离与回收机制只退役已确认归当前会话独占的 provider。确保旧连接退出或被隔离、最终状态落盘后才允许启动同一会话的新轮。迟到事件按回合及运行代次丢弃，未确认结束的旧连接禁止进入复用池。
5. 保持现有持久化收据与并发校验为交付依据。优先复用当前 SQLite 字段及含义，避免给既有队列和 Tauri 的共享格式增加迁移成本。若回归证明需要持久化额外状态，先补充兼容及重启场景的设计。

## 验证与部署边界

模拟 provider 可确定性复现等待、迟到回执及无停止确认。Windows 上用同一构建验收队列状态、会话恢复和其他会话并行推进。以最新主线为集成基线，使用已有的 Claude 后台任务测试对照。产品变更以代码和测试交付，安装版回归在独立开发数据目录完成。

Windows 的进程树回收与管道线程收尾可能阻塞。空闲进程池中的移出和进程回收分两步，先在锁下移出，再释放共享锁并清理进程。模拟并发用例同时覆盖某一会话的 Stop 超时和另一会话的追问送达。
