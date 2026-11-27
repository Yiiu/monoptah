# 第1期：状态准确（后台任务 + Windows 提醒）

Parent: `11-25-agent-overview`（共同原则和完整需求见父任务 `prd.md`）。这是第一个交付，其他三期都依赖本期的状态模型。

## Goal

每个 agent 会话的状态，在所有界面上都准确且一致：不再出现只挂着 dev server 却显示 Working，也不再出现状态卡住不灭。在 Windows 上，窗口不在前台时，需要用户处理的会话能被注意到。

## Requirements

### R1 统一的状态模型

每个会话由同一个函数推出一份状态，所有界面只读这一份结果：

| 状态 | 条件 |
|---|---|
| NeedsYou | 有待回答的问题或待批准的请求（含远程会话上报的等待） |
| Running | 这一轮正在输出 |
| BackgroundWait | 这一轮已结束，但还有会把 agent 叫醒的后台工作（非 ambient 的子 agent / workflow / monitor）；或者额度用完、等待自动恢复 |
| Done | agent 停下了。结果是完成、失败或被中断之一，并带一个已读 / 未读标记 |

- 优先级：NeedsYou > Running > BackgroundWait > Done。
- 后台 shell 任务（dev server、watch、后台测试）不影响状态，作为单独的列表输出，供界面显示一个标签（悬停可看命令）。
- 状态附带：当前步骤（最近一次工具调用，例如 "Read src/app.rs"）、进入当前状态的时间。终端会话和远程会话拿不到步骤，只给时间。

### R2 后台任务的正确性（Claude）

- 只挂着后台 shell 任务时，这一轮要正常结束，进入 Done。
- 后台任务的列表以 Claude 每次给出的完整列表为准；任务从列表中消失后，不能有残留把状态卡住。
- 后台任务结束、Claude 自己醒来继续时，状态回到 Running。
- 后台任务的变化要立即反映到界面，不用等其他事件触发。
- Codex（没有后台任务）和 Hermes（自己接手后台任务）的行为不变，只是走统一的状态函数。

### R3 已读 / 未读

- 判定为看过，需要同时满足：该会话正显示在屏幕上、窗口在前台、最近 3 分钟内有鼠标或键盘操作。点击通知打开会话也算看过。
- 已读状态跨重启保留。
- 升级后第一次启动时，所有已有会话视为已读。

### R4 防卡死

- 重启后，原来在跑、但进程已经不在的会话，显示为 Done（被中断）。
- Running 超过 30 分钟没有收到任何事件时，显示"No update in 30m"，不再转圈。

### R5 现有两种侧边栏布局用上新状态

只改状态文字和图标，不改布局：
- Running：显示当前步骤 + 时长，例如 "Running tests · 4m"；拿不到步骤时显示 "Working · 4m"；最多约 15 秒刷新一次。
- BackgroundWait：显示 "Waiting on 2 subtasks"，或者额度用完时显示 "Usage limit · resumes in 23m"。
- Done：未读时显示 "Done"（失败为 "Failed"，被中断为 "Stopped"）；已读后只显示相对时间。
- 有后台 shell 任务时，额外显示一个小标签。

其他所有界面也都读同一份状态：Live agents 卡片、标签页、筛选、通知、Mono 面板、Today / Inbox 的运行中计数、手机端目录（`busy` / `waiting` 字段按新状态换算，格式不变）。

界面文字沿用应用现有的语言（英文）。

### R6 Windows 提醒（暂缓：2026-11-25 用户决定本期不做，保留需求供以后单独立项）

窗口不在前台时，某个会话进入 NeedsYou：
- 弹出 Windows 系统通知：标题为"项目 · 会话"，正文为 agent 的原话，带「Open」按钮；点击直接进入该会话，并算作看过。
- 任务栏图标叠加数字角标，数字是 NeedsYou 的会话数；数字变化时随之更新，为 0 时清除。
- 任务栏按钮闪烁，直到窗口回到前台。
- 播放提示音。
- 同一个会话 5 秒内不重复弹通知。
- 窗口在前台、并且正看着那个会话时不弹通知。
- Done 只更新角标计数，不弹通知（角标只数 NeedsYou，所以 Done 不改变数字）。

其他要求：
- 开发版和正式版的通知互不干扰（各自独立的应用身份）。
- 不新增任何设置项。现有的通知开关和静音规则继续生效。

## Acceptance Criteria

- [ ] 一个只剩后台 dev server 的 Claude 会话，这一轮结束后显示 Done，并带 shell 标签；dev server 继续运行。
- [ ] 后台子 agent 在跑时显示 "Waiting on N subtasks"；子 agent 完成、Claude 醒来后回到 Running，最终 Done。
- [ ] 后台任务从 `background_tasks_changed` 列表里消失（没有对应的 `task_notification`）后，状态不会卡住。
- [ ] 所有界面在同一时刻对同一会话给出一致的状态：现有两种侧边栏、Live agents、标签页、筛选、通知、Mono 面板、Inbox 计数、手机端目录。
- [ ] 未读的完成在重启后仍然是未读；升级后第一次启动没有满屏的未读标记。
- [ ] 窗口在前台但用户离开超过 3 分钟时，新完成的会话保持未读。
- [ ] 重启后，原来在跑、但进程已经不在的会话显示 Stopped，不显示 Working。
- [ ] 没有新增设置项；macOS 现有的通知行为不变。
- [x] PR #22 的 `4c3603c1` 通过 Linux CI（fmt、clippy `-D warnings`、GPUI 测试）和 Windows `cargo check`。[CI 记录](https://github.com/Yiiu/monoptah/actions/runs/45747487531)

本期 R1–R5 由 `feat/agent-overview-tree` 分支（PR #22）交付。R6 由 `11-26-windows-attention` 跟进。

## Out of scope

- 项目树侧边栏、进展摘要、归档（第2期）。
- Mono 相关改动（第3、4期）。
- 终端会话改用 hook 获取步骤信息。
- 「标为未读」。
- 应用退出后从通知中心点击重新启动应用（需要 COM 激活器）。
