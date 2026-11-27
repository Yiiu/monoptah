# 同类工具的会话追问、排队和停止

调研日期为 2026-11-26。比较本地会话交互和公开协议。以下结论依据产品文档与公开源码，运行行为仍需用 Monoptah 的 Windows 测试验证。

## 同类工具

| 参照 | 可核实的行为 | Monoptah 可借鉴 |
| --- | --- | --- |
| Orca Native Chat | 待发消息在主机确认前持续显示正在发送。明确失败带原因和重试入口。重启后核对 provider 历史，无法确认是否送达时保留未确认状态。Codex 拒绝停止或没有回答时，Orca 会结束对应进程。 | 首选参考。保留每条消息的交付结论。确认缺失时通过历史核对，明确未送达后允许重试。退出应用前后的恢复语义一致。详见 [Orca 官方说明](https://orca.dev/docs/agents/native-chat#composer)及本任务的 [Orca 专项核对](orca-follow-up-recovery.md)。 |
| T3 Code | 明确区分 Queue 与 Steer，Stop 后暂停队列并保留顺序。旧轮结束或恢复时检查仍待处理的 Steer。Stop 超时后可按原 run 强制停止。 | 用队列项保留原文与操作意图，停止恢复入口绑定目标回合。超时按缺少回执和缺少终态分别验证。对交付未知的消息用 Orca 的历史核对方式。[T3 Code 专项核对](t3-code-session-controls.md) |
| VS Code Chat | 提供 Add to Queue、Steer with Message 和 Stop and Send。Steer 等当前工具执行结束后切换消息，Stop and Send 先取消当前请求再发。Agent Debug Logs 按时间记录事件。 | 显式区分等下一轮、调整当前轮和停止后发送。把当前轮是否结束作为启动下一轮的门槛。调试日志沿用事件时间线的思路，Monoptah 的日志仅收录脱敏阶段状态。[会话操作](https://code.visualstudio.com/docs/chat/chat-overview#send-messages-while-a-request-is-running) · [调试日志](https://code.visualstudio.com/docs/agents/agent-troubleshooting/chat-debug-view) |
| Claude Code CLI | 工作期间显示已排队条目；普通消息可在工具调用结束后进入同一轮，待发项还可在该轮结束后按顺序送达。近期 CLI 的发送快捷键可将支持的工具任务移到后台，其他情形会中断后发送。 | 文案允许存在安全等待点。不同代理各用其可验证的接收和终态信号。Claude CLI 能后台化的工作需由该 CLI 确认。[排队和发送时机](https://code.claude.com/docs/en/interactive-mode#queue-messages-while-claude-works) |
| Zed Agent Panel | 默认等当前轮结束再发队列。Zed 自有代理可在下一步插入 Steer；外部代理的 Steer 受会话边界能力限制。提供明确的 Stop 和 Send Now 操作。 | 是否允许 Steer 取决于当前回合可用的传输能力和状态。客户端未派发时把原因呈现给用户，并保留普通排队与停止后发送入口。[Zed 消息队列](https://zed.dev/docs/ai/agent-panel#queueing-messages) |

VS Code 所述的 Steer 是完成当前工具调用后让当前请求让位。Claude Code 的 CLI 能把部分工作移至后台。这些属于各自宿主的行为，Monoptah 的 Codex 适配以 App Server 回执和终态事件为实现依据。

## Codex App Server 协议约束

- `turn/steer` 携带当前 `expectedTurnId`，成功返回接收这条输入的 `turnId`。客户端依靠请求 ID 区分不同尝试。回执说明 Codex 接收了输入，模型在后续步骤实际使用该消息需要观察回合内容。没有活动轮时该调用明确失败。[Steer 协议](https://learn.chatgpt.com/docs/app-server#steer-an-active-turn)
- 现场使用的 Codex CLI 0.172.0 已支持可选的 `clientUserMessageId`。本机通过该版本的 `app-server generate-json-schema` 核实稳定协议输出包含此字段；对应版本的源码测试验证 `item/started` 的 `userMessage.clientId` 与传入 ID 一致。它适合将界面待发项和 provider 事件关联起来。上游仍有为 Steer 补充幂等重试的提案，超时时应根据事件或持久化历史核对，避免直接重发。[0.172.0 协议结构](https://github.com/openai/codex/blob/rust-v0.172.0/codex-rs/app-server-protocol/schema/typescript/v2/TurnSteerParams.ts) · [0.172.0 集成测试](https://github.com/openai/codex/blob/rust-v0.172.0/codex-rs/app-server/tests/suite/v2/turn_steer.rs#L331-L387) · [上游幂等提案](https://github.com/openai/codex/issues/32254)
- `turn/interrupt` 的成功响应是空对象。回合结束由随后带 `interrupted` 状态的 `turn/completed` 表示。若无法等到该终态，只在用户明确停止后对目标会话做有界收尾，并验证旧连接已隔离再开新轮。[中断协议](https://learn.chatgpt.com/docs/app-server#interrupt-a-turn)
- `thread/status/changed` 能提供活动状态和等待审批等信息，用于解释等待原因并关联诊断。交付凭据仍按消息和回合核对。[线程状态事件](https://learn.chatgpt.com/docs/app-server#track-thread-status-changes)

## 对任务方案的建议

1. 优先采用 Orca 的交付确认与单会话恢复原则，沿用 Monoptah 的待发项和收据。分别显示本地排队、派发等待、代理接收、明确失败和接收结果未知。支持时把已有待发项 ID 作为 Codex 的 `clientUserMessageId`，用 RPC 回执或实际用户消息事件核对。超时后继续接收迟到确认，只有明确未送达时才开放重试。
2. 停止由用户触发。等待确认与等待终态分别计时。超时收尾只作用于确认为本地独占的 provider，按运行代次隔离晚到事件。对于共享传输的情形，先验证会话级取消能力。
3. 参考 VS Code 的操作语义澄清现有队列和 Steer。“停止并发送”可作为现有入口的后续收敛，在单会话恢复和去重验收通过后再确定是否合并成一键。它必须沿用同一个待发项。
4. 针对已观察到的 Windows 现场，验收应覆盖工具仍在跑时的正常等待、没有返回确认、晚到确认、原回合自然结束后发下一轮。另测 Stop 成功回执与真正终态之间的间隔。两个路径都要证明历史、队列和并行会话可继续使用。

调研能确定产品交互先例和协议边界。历史现场仍缺少请求是否写入 Codex 的事件，所以实施先补齐阶段证据并用可控 provider 重放。
