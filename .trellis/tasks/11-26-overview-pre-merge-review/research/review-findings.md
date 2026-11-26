# PR #22 合并前复核

复核对象为 `22714cf7...4c3603c1`。源代码行号对应 `4c3603c1`。已合并的后续分支需要用各自的回归和验收结果关闭下列事项。

| 范围 | 结论 |
| --- | --- |
| 迁移与重启清理 | 首次回填事务和两项可空列升级是加法，正常重启保留已读水位。丢表重建重复执行首次回填，是合并 blocker。Windows 无法确认进程身份时有保守退化。 |
| 数据保留 | 自动归档保留会话内容并按工作树路径检查资格，后台结果回到界面时再次检查布局与会话状态。归档和消费证据的分开写入存在崩溃窗口。重启中断会保存该轮的保底摘要。 |
| 定时器与 CPU | 流式输出每批重新读取步骤，Mono 时钟在巡检到期判断之前创建视图，额度等待固定唤醒，Tree Git 检查在失焦后仍运行。均已分派到状态核心任务。 |
| Mono 决策卡与巡检 | 旧卡片可能错误进入 Needs you，选项和存储增长已分派到状态核心任务。巡检对会话状态和工作树做快照，成功后确认基线，失败会保留已确认基线。 |

## Blocker

### 首次已读迁移可能重复运行

- 证据 `crates/monocode-core/src/store/session_activity.rs:29–61`、`crates/monocode-gpui/src/shell/agent_activity.rs:53–60`。已知事项见状态审计第 21 项和 §b.1。
- 场景：已有一个完成未读的会话，`session_activity` 表被删除。重启或下一次写入时，代码以缺表判断首次升级，再给现存会话写入已读水位。运行中尝试恢复时还会丢弃数据库返回的记录，界面和持久化状态可能不同。
- 修复：在表以外持久化迁移完成标记。测试分别覆盖运行中修复以及重启后的重建，确认未读回执可恢复。进程退出后只保留在内存中的回执会随进程消失，需要依据仍然存在的持久化数据恢复。
- 归属 `11-26-status-perf-correctness` R6。

## Should

### 归档已落库，消费证据还未落库

- 证据 `crates/monocode-gpui/src/shell/progress_summary.rs:360–375,392–418,437–461`、`crates/monocode-gpui/src/shell/agent_activity.rs:43–45,62–74`。
- 场景：Tree 自动归档的 `set_archived` 已写入数据库，应用在界面回调提交证据之前退出，或侧车写入持续失败。用户重启后手动恢复会话，同一个合并提交或移除事件还会将它归档。若程序在用户切换布局或重新开始回合后的补偿写入之前退出，暂时的归档标记也会保留。
- 修复建议：在存储层使用可恢复的归档操作，令归档与消费证据共同提交或持久化一个可重放的意图。覆盖状态写入成功、证据写入失败与同一证据下的手动恢复。约束横跨共享会话记录和 Native 侧车。
- 归属 `11-26-status-perf-correctness` 的自动归档调用链。此事项可独立于 R2 的轮询调整处理。

### Windows 查不到进程信息时无法识别 PID 已复用

- 证据 `crates/monocode-core/src/store/session_activity.rs:355–368,414–426,453–487`。已知事项见状态审计第 7 项和 §b.2。
- 场景：旧 Native 进程已退出，PID 被受保护的进程复用。`OpenProcess` 返回访问拒绝，未知启动时间按匹配处理，原回合不会被恢复成 Stopped。
- 修复建议：区分确认存在、确认退出和无法确认身份，保留访问拒绝时保护可能仍在运行的 Native 进程的行为。只有额外的本机进程身份或启动时间证据足够时才收回该回合。
- 归属 `11-26-status-perf-correctness` 的重启回执处理。

### 活动计时和巡检仍有空闲开销

- 证据 `crates/monocode-gpui/src/conversation.rs:508–511,2089`、`crates/monocode-gpui/src/transcript.rs:1085–1120`、`crates/monocode-gpui/src/shell/agent_activity.rs:111–143`、`crates/monocode-gpui/src/shell/live_agents.rs:86–109`、`crates/monocode-gpui/src/monos/scheduler.rs:1485–1541`、`crates/monocode-gpui/src/shell/progress_summary.rs:191–205,238–264`。已知事项见状态审计第 3–5 项和 §b.3。
- 归属 `11-26-status-perf-correctness` R1、R2。流式步骤读取随工具调用更新，先检查巡检是否到期，再构建会话视图，按额度恢复时间调整唤醒间隔，并在窗口失焦时跳过 Tree Git 检查。
- 空闲边界：没有运行和额度倒计时时，状态时钟会退出。Mono 的 10 秒调度仍负责既有习惯与时间标签。已打开 Mono 的 `shown_fingerprint` 另有一条全会话视图读取（`scheduler.rs:1575–1603`），性能验证应区分该调用与巡检快照。Tree 在有符合条件的会话时可周期性运行 Git，失焦后当前只检查布局。

### 旧决策卡与增长限制需要一起验证

- 证据 `crates/monocode-gpui/src/transcript/mono_decisions.rs:19–76,100–123`、`crates/monocode-gpui/src/monos/cards.rs:456–487`、`crates/monocode-core/src/mono/cards.rs:129–134`、`crates/monocode-core/src/mono.rs:71–72`、`crates/monocode-gpui/src/monos/scheduler.rs:1507–1538`。已知事项见状态审计第 2、10、20 项和 §b.4。
- 归属 `11-26-status-perf-correctness` R4、R7。旧卡片后面的普通用户消息应结束升级前的等待；新卡仍按卡片 ID 跟踪答案。验证裁剪已答记录后的回读，因为该卡落入旧记录推断后，`Decision on …` 文本不会等于单个选项标签。新选项超限时应返回可见的工具错误。巡检从快照移除已归档会话。

## 其余路径核对

- 常规迁移使用独立 Native 侧车，先取得 IMMEDIATE 事务再探测表和列。只读库在进入迁移前被拒绝，内存测试回退到测试数据库。现有 `migration_marks_existing_history_read_once_and_leaves_drafts_idle` 覆盖正常的再次启动。
- 清理设置页以实际移除命令成功后、按路径匹配的 `detach_worktree` 返回值为依据。周期探测按项目、保存的执行路径和 Git 工作树归属检查。归档前排除带草稿、活跃、待回答、远程和终端会话，然后重新检查 Git 证据、存储中的路径和本地位置。归档只改标记，保留会话历史。
- `session_activity.rs:370–379` 只在确认失去本地进程所有者的回合上把旧摘要替换为 `Stopped before finishing`。已完成回合的所有者字段为空，不参与恢复。每个会话仅存最新结束回合的摘要，所以这是新中断回合的保底文本。
- 巡检以 `last_run_at` 并发检查认领批次，`heartbeat.rs:64–93` 在成功确认前保留上一份基线。`scheduler.rs:2024–2053` 在报告入队保存成功后确认批次，也支持设计允许的静默成功。

## 验证

- `git diff main...feat/agent-overview-tree --stat` 退出码 0，初始变更涉及 170 个文件。
- `gh api repos/:owner/:repo/commits/4c3603c1c7e10151ee1ca9498e8bc1665eedee2e/check-runs` 退出码 0。Linux Core、GPUI lint、GPUI 测试三分片、Windows GPUI check、Windows Core 测试三分片及 CI 汇总均为 `success`。[CI 记录](https://github.com/Yiiu/monoptah/actions/runs/45747487531)
- 用 Python SQLite 内存库执行从 `session_activity.rs` 提取的建表和首次回填 SQL。退出码 0，`DROP TABLE` 前回执为 `turn=110 seen=100`（未读），重建后为 `turn=110 seen=120`（已读）。R6 的回归覆盖应用层重建。
- 其他复核为源码和已有测试的静态检查。此次提交仅更新复核报告及两份 PRD。
