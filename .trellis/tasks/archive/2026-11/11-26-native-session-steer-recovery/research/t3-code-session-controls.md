# T3 Code 的排队、追问与停止

2026-11-26 核对了 T3 Code 的公开产品文档、`main` 固定提交及相关 PR。[T3 Code 提交 6ac4ede](https://github.com/pingdotgg/t3code/commit/6ac4edef8cd02ec655b999d213f18e149171a867) · [T3 Code 源码核对](t3-source-verification.md) · [Orca 专项核对](orca-follow-up-recovery.md)

## 已核实的行为

| 维度 | T3 Code | Orca |
| --- | --- | --- |
| 忙时输入 | Queue 存在服务端，等待下一轮。支持 Steer 的代理可把消息送进当前轮。Windows 可用 Ctrl+Enter 临时切换发法，Ctrl+Shift+Enter 追问最早的队列项。Stop 或服务端重启后按原顺序暂停。 | 队列允许编辑、暂停、恢复和选择单条发送。Steer 发送后等待宿主确认。 |
| 发送确认与失败 | Codex Steer 校验目标回合并等待 RPC。识别出该轮已结束或明确拒绝时，转入后续派发。旧轮终结或恢复时检查仍标为待处理的 Steer。已有 Stop 记录时提示“未确认送达”，其他情况可按原消息 ID 重新排队。 | 通过 RPC 和代理消息回显关联交付。重连或重启后用代理历史核对，无法确定时保留未确认，明确未送达时提供重试。 |
| 停止不收尾 | provider 的中断调用有 15 秒期限，包含 RPC 的等待。Codex 收到 RPC 回复后另等回合终态，内层最多 10 秒。无响应可提供 Force Stop，按原 run 校验，并保留其他会话正在使用的共享进程。 | 用户点 Stop 后，在有界等待内尝试中断并收尾目标写入端。继续派发前检查退出或隔离证据，旧写入端的状态无法确认时继续保留互斥。 |

- 队列还支持编辑、移除和忙时立即发送。Resume 在支持原生恢复的 provider 上继续处理，无需合成新的用户提示词。[忙时输入](https://github.com/pingdotgg/t3code/blob/6ac4edef8cd02ec655b999d213f18e149171a867/docs/user/composer.md#L33-L45) · [排队和恢复](https://github.com/pingdotgg/t3code/blob/6ac4edef8cd02ec655b999d213f18e149171a867/docs/user/composer.md#L117-L125)
- 15 秒的中断期限来自会话管理器，10 秒的终态等待属于 Codex 适配器。用户看到的 Force Stop 检查该次操作对应的 run，并在共享进程或回收未结束时给出提示。上述期限针对 provider 中断，Codex 的常规 `turn/steer` 未发现对应的回执期限。[中断期限](https://github.com/pingdotgg/t3code/blob/6ac4edef8cd02ec655b999d213f18e149171a867/apps/server/src/orchestration-v2/ProviderSessionManager.ts#L84-L87) · [中断与 Steer 包装层](https://github.com/pingdotgg/t3code/blob/6ac4edef8cd02ec655b999d213f18e149171a867/apps/server/src/orchestration-v2/ProviderSessionManager.ts#L2766-L2790) · [Codex 终态等待](https://github.com/pingdotgg/t3code/blob/6ac4edef8cd02ec655b999d213f18e149171a867/apps/server/src/orchestration-v2/Adapters/CodexAdapterV2.ts#L9236-L9278) · [Force Stop 操作](https://github.com/pingdotgg/t3code/blob/6ac4edef8cd02ec655b999d213f18e149171a867/apps/web/src/components/chat/useForceStopThread.ts#L17-L61) · [按 run 校验与回收](https://github.com/pingdotgg/t3code/blob/6ac4edef8cd02ec655b999d213f18e149171a867/apps/server/src/orchestration-v2/ThreadManagementService.ts#L1450-L1492)
- worker 用 `turnCompleted` 或 `steerDeclined` 触发后续发送。另一路在旧轮终结或恢复时检查待处理的用户输入，根据 Stop 记录提示或者尝试重新排队。扩展 Claude 崩溃后补发且保留暂停队列顺序的 PR 查询时仍开放。Monoptah 对交付结果未知的消息采用 Orca 的历史核对策略。[Codex Steer 调用](https://github.com/pingdotgg/t3code/blob/6ac4edef8cd02ec655b999d213f18e149171a867/apps/server/src/orchestration-v2/Adapters/CodexAdapterV2.ts#L8717-L8788) · [明确拒绝后的处理](https://github.com/pingdotgg/t3code/blob/6ac4edef8cd02ec655b999d213f18e149171a867/apps/server/src/orchestration-v2/EffectWorker.ts#L365-L407) · [待处理 Steer 的恢复](https://github.com/pingdotgg/t3code/blob/6ac4edef8cd02ec655b999d213f18e149171a867/apps/server/src/orchestration-v2/Orchestrator.ts#L3575-L3714) · [PR #37992](https://github.com/pingdotgg/t3code/pull/37992)

## 对 Monoptah 的取舍

1. 优先把 Steer 的等待、明确受理、明确拒绝与结果未知呈现在原队列项上。等待超时继续接收晚到证据。当前轮结束时，仅按已有证据决定消息是否可进入下一轮。应用仍标记待处理，只能说明应用尚未看到消费记录。保持任务已有的去重与历史核对要求。
2. Stop 分别覆盖中断请求无回执和有回执但无终态。先回收或隔离当前会话旧写入端，再允许新轮。若仍需用户操作，参考 T3 将恢复入口绑定原 run，避免迟到的点击影响新一轮。
3. `resume_queue` 的 Native 本地路径目前先发送 `Continue from where you left off.`，再恢复队列。后续可把“发送待发消息”和“继续原任务”做成不同操作，从而让只想补一句话的用户直接送出原消息，避免额外启动已停止的工作。[Monoptah 恢复队列](../../../../crates/monocode-gpui/src/conversation/follow_up.rs)

本次核对采用源码和产品文档。交互时延、Windows 恢复路径和现场 Steer 首个阻塞点仍纳入 Trellis 复现。
