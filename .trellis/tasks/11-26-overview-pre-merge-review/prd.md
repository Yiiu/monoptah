# 合并前独立复核

Parent: `11-25-agent-overview`。开工前先读父任务的 `handoff.md`。

## Goal

对 PR #22 做一次独立复核。不要求通读全部代码，只抓合并后会伤到用户数据、拖慢应用或行为出错的地方。仓库规则（CLAUDE.md "Review once per request"）要求数据库迁移、删除数据这类高风险改动单独复核，而这个 PR 里有些部分从来没有被单独看过。

## Scope

复核的对象是 `feat/agent-overview-tree` 相对 `main` 的完整 diff，重点是下面四块：

1. **迁移与重启清理**：`crates/monocode-core/src/store/session_activity.rs`、`session_activity_tests.rs`，以及 GPUI 侧的调用方 `shell/agent_activity.rs`。
   - 迁移是否只做加法、可以重复执行；第一次启动时"全部标为已读"是否只发生一次。
   - 表不存在时会走重试路径（审计第 21 项），这条路径会不会再次把所有会话标为已读。
   - 重启清理的归属判断：按 pid + 启动时间。Windows 上 `OpenProcess` 失败一律当作"进程还活着"处理（审计第 7 项），这样合不合理。
2. **数据丢失**：`shell/progress_summary.rs:191-330`。
   - 合并或清理 worktree 后会自动归档。要确认只归档真正属于那个 worktree 的会话，而且只在 Tree 布局下触发。
   - 重启清理会不会覆盖掉已有的进展摘要。
3. **定时器与 CPU 开销**：
   - 每次流式更新都刷新状态，约 16ms 一次，每次都会扫描 transcript 并解析 JSON（审计第 3 项）。
   - Mono 时钟每 10 秒为所有 Mono 构建一遍会话视图（审计第 4 项）。
   - 额度等待期间 15 秒一次的计时器会一直跑，可能持续几个小时（审计第 5 项）。
   - Tree 布局下每 75 秒检查一次 git worktree。
   - 需要判断：哪些开销在空闲时也停不下来。
4. **Mono 决策卡与巡检**：`transcript/mono_decisions.rs`、`core/mono/cards.rs`、`core/mono/patrol.rs`、`monos/scheduler.rs`。
   - 升级前留下、但一直没回答的旧选项卡，会不会错误地显示成 Needs you（审计第 2 项）。
   - 选项数量现在没有上限（审计第 10 项）。
   - `decision_cards` 和 `heartbeat.json` 会不会无限增长（审计第 20 项）。

同时把过时的任务文档改正确（审计第 19 项）：
- `11-25-agent-overview/prd.md`、`11-25-agent-status-core/prd.md` 里写的分支名改为 `feat/agent-overview-tree`；
- "Linux CI 未验证"这类过时表述按实际情况更新（CI 已通过）。

## Output

- `research/review-findings.md`：每条发现都要写清楚以下内容：
  - 严重程度：blocker / should / nice
  - `file:line`
  - 具体的失败场景，即什么输入或状态会导致什么错误结果
  - 修复建议
  - 归属哪个后续任务：`status-perf-correctness`、`tree-polish`、`tree-motion-followups`，或"复核者直接修"
- 如果某条发现和审计里已有的条目相同，标注是审计里的第几项，不要重复展开。
- 小而独立的修复（一两个文件、有测试兜底）由复核者直接提交，并在 findings 里注明对应的 commit。

## Acceptance Criteria

- [ ] 上面四块都给出明确结论：没问题，或者列出具体发现。
- [ ] 每个 blocker 都有修复方案，并且已经修复，或者已经分配给某个后续任务。
- [ ] 任务文档里的分支名和 CI 状态已经改正确。
- [ ] 只在需要证明某个具体发现时才编译或跑测试（CLAUDE.md 的复核规则）。
