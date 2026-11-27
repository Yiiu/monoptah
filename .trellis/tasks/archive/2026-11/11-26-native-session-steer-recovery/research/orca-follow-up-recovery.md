# Orca 本地会话的追问、排队与停止恢复

- 调研问题为本地 Codex 和 Claude 工作中如何接收追问、确认送达、处理失联并恢复发送。
- 范围为公开文档、固定版本源码及 Monoptah 当前调用链。
- 调研日期为 2026-11-26。Orca 源码固定在 [`362ecf36`](https://github.com/stablyai/orca/commit/362ecf36396015b1425610ebd56ad81401c4872d)。Codex 兼容性对照现场使用的 [`rust-v0.172.0`](https://github.com/openai/codex/releases/tag/rust-v0.172.0)。

## 用户可见行为

- 默认把普通追问放在输入框上方的可编辑队列。Steer 把选中的消息交给当前轮，失败后队列暂停，用户可 Resume 或单独发送一条。草稿和附件可以跨断连保留。[Orca Chat UI](https://orca.dev/docs/agents/native-chat#composer)
- 等待宿主确认时显示发送中，明确失败时显示原因并允许手动重试。重启时检查代理记录，能证明送达就补记，能证明未送达才开放重试，结果不明确时保留未确认状态。中断后展示原因并提供 Continue，由下一轮检查上一步有没有完成。[Orca Chat UI](https://orca.dev/docs/agents/native-chat#composer)

## 源码可复核行为

| 位置 | 行为 |
| --- | --- |
| [`src/main/codex/structured/turns/codex-structured-turn-steer.ts:27–72`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/codex/structured/turns/codex-structured-turn-steer.ts#L27-L72) | 写入 `turn/steer` 前检查当前派发权限，登记预期回显。请求带 `threadId`、`expectedTurnId` 与稳定的 `clientUserMessageId`。 |
| [`codex-structured-turn-start.ts:227–269`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/codex/structured/turns/codex-structured-turn-start.ts#L227-L269)、[`codex-structured-dispatch-echo.ts:310–324`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/codex/structured/turns/codex-structured-dispatch-echo.ts#L310-L324)、[`codex-dispatch-receipt-correlation.ts:112–162`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/codex/structured/turns/codex-dispatch-receipt-correlation.ts#L112-L162) | JSON-RPC 回复证明获准派发。用户消息回显优先从 `clientId` 对账。缺少 ID 时，结合会话、回合、内容和派发先后顺序保守配对，冲突时保持未确认。 |
| [`codex-app-server-connection.ts:140–179`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/codex/app-server/codex-app-server-connection.ts#L140-L179)、[`structured-agent-session-send-settlement.ts:51–89`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/native-chat/agent-session-wire/delivery/structured-agent-session-send-settlement.ts#L51-L89)、[`structured-agent-session-late-dispatch.ts:9–68`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/native-chat/agent-session-wire/operations/structured-agent-session-late-dispatch.ts#L9-L68) | 通用 Codex RPC 默认等待 30 秒并允许调用方覆盖。宿主确认另有等待预算与消息身份。迟到证据可更新日志，只有明确未执行的队列消息才回到可发送状态。 |
| [`src/main/claude/structured/dispatch/claude-structured-dispatch.ts:146–176`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/claude/structured/dispatch/claude-structured-dispatch.ts#L146-L176)、[`claude-structured-dispatch.ts:233–259`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/claude/structured/dispatch/claude-structured-dispatch.ts#L233-L259) | Claude SDK 传输写入带 UUID 的用户消息。写成功先记入待确认，回显后结算。可证明从未写入时判定拒绝，写入结果含糊时记未知。 |
| [`structured-agent-session-stop-acceptance.ts:81–132`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/native-chat/agent-session-wire/operations/structured-agent-session-stop-acceptance.ts#L81-L132)、[`structured-agent-session-stop-wind-down.ts:29–81`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/native-chat/agent-session-wire/termination/structured-agent-session-stop-wind-down.ts#L29-L81)、[`structured-agent-session-child-close.ts:55–91`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/native-chat/agent-session-wire/termination/structured-agent-session-child-close.ts#L55-L91) | Stop 先持久化操作和目标，再中断并收尾。关闭绑定原 provider，后续派发等待退出或终止信号已送达的证据。无法验证旧进程退出时仍保留互斥。 |
| [`codex-request-end-wait.ts:4–45`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/codex/structured/turns/codex-request-end-wait.ts#L4-L45)、[`claude-request-end-wait.ts:5–64`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/claude/structured/turns/claude-request-end-wait.ts#L5-L64)、[`codex-shared-server-writer-stop.ts:5–52`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/codex/app-server/shared-servers/codex-shared-server-writer-stop.ts#L5-L52) | 各层等待预算不同。Codex 适配器 Stop 预算为 5 秒，Claude 为 3 秒，共享 Codex 写入端另有 10 秒终态证明等待。这些常量不能直接当成用户从点击到恢复的时长。 |
| [`codex-shared-server-hang-watch.ts:5–55`](https://github.com/stablyai/orca/blob/362ecf36396015b1425610ebd56ad81401c4872d/src/main/codex/app-server/shared-servers/codex-shared-server-hang-watch.ts#L5-L55) | 共享 app-server 的失联检测要求发出请求后既无回复又无其他入站帧。达到 120 秒才回收共享 server，挂在上面的会话会在下一次发送时连接新 server。 |

## 版本兼容性

Codex 0.172.0 的生成协议已经把 `clientUserMessageId` 列为可以省略或为 `null` 的输入字段。上游集成测试发送该字段后，检查 `item/started` 的 `userMessage.clientId` 关联到同一条消息。留出旧版、字段缺失、消息回显没有出现的兼容路径。这个 ID 支持关联证据，当前材料未证明服务端会阻止重发相同 ID 时重复执行。[0.172.0 schema](https://github.com/openai/codex/blob/rust-v0.172.0/codex-rs/app-server-protocol/schema/json/v2/TurnSteerParams.json#L310-L335) · [0.172.0 集成测试](https://github.com/openai/codex/blob/rust-v0.172.0/codex-rs/app-server/tests/suite/v2/turn_steer.rs#L331-L387)

## 与 Monoptah 的连接

- `crates/monocode-gpui/src/conversation/follow_up.rs:320` 在现有 `pump_follow_ups` 的门槛处记录阻塞原因，在队列行显示等待派发、等待确认和结果未知。
- `crates/monocode-core/src/provider/follow_up.rs:53–60,123–174` 目前生成的是每次发送都不同的 RPC 请求 ID，以回复驱动结算，Codex 不参与现有确认超时分支。可沿用 RPC 请求 ID 匹配回复，并用稳定的队列消息 ID 关联支持该字段的 Codex 用户消息回显。
- `crates/monocode-core/src/provider.rs:1902,1989` 与 `crates/monocode-gpui/src/conversation.rs:1922` 在实际收到 Stop 后采用单会话收尾预算，分别验证中断回复、旧回合完成和 provider 退出。正常长工具持续产生进度时照常运行。
- 相关规范为 `.trellis/spec/core/processes-and-platforms.md`、`.trellis/spec/core/persistence-and-compat.md` 与 `.trellis/spec/native/state-and-async.md`。

## 证据边界

源码与文档支持上述交互语义，实际 Windows 端恢复时延和 Monoptah 现场的首个阻塞点仍需复现。Monoptah 本地当前会话可以按其已绑定的 provider 回收，Orca 的共享 app-server 失联恢复可能影响多个会话，其架构专有的行为不宜直接移植。
