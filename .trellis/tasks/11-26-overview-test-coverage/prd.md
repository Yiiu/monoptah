# 补齐测试与三种外观验证

Parent: `11-25-agent-overview`。开工前先读父任务的 `handoff.md`。这个任务**最后做**，前面几个任务都合回主分支后再开始，这样测试针对的是最终行为。条目来源：`audit-status-mono.md` 的第 13–18 项，以及 `audit-tree-summary.md` 的第 8 项。

## Goal

功能已经做好，但下面这些行为还没有测试覆盖，这次全部补上。另外，三种外观要实际打开应用逐一核对一遍。

## Requirements

### 需要补的测试
1. **Hermes 的状态映射**：Hermes 的后台任务要映射成 BackgroundWait；后台任务结束后，状态回到 Running，最后变成 Done（`core/provider/acp.rs:920,1116`）。
2. **点击通知算作已读**：点击通知打开会话后，该会话的 `seen_at` 要更新，行也要变成已读样式（`shell/notifications.rs:695,747`）。
3. **Classic 和 Focused 布局里的行文字**：要覆盖 "Failed"、"Stopped"、"Waiting on N subtasks"、"Usage limit · …"、"No update in 30m" 这几种文字，以及后台 shell 小标签的显示。
4. **手机端目录的映射**：`busy` 等于 Running 或 BackgroundWait；`waiting` 取自 NeedsYou；输出的 JSON 格式和之前完全一致（`shell/mobile_host.rs:86-102`）。
5. **Mono 派出的会话进入项目树**：会话挂在对应的 worktree 下，标题后面显示该 Mono 的像素小人（`shell/project_tree.rs:495`、`render.rs:782-787`）。
6. **Mono 计划清单**：「N done / N running / N needs you」的数字要直接从它派出去的会话推算（`monos/dashboard.rs:274-460`）。

### 三种外观的实机核对
- 在 Native Dev 里依次切换到深色玻璃（默认：侧边栏不透明度 85%、玻璃正文开启）、深色不透明、浅色三种外观，每种都截取 Tree 布局下的这些部分：项目树（包含在跑、等你、完成未读、失败几种状态）、收起状态栏、从左侧滑出的面板、标签页的状态图标。
- 对照定稿设计稿检查：所有颜色都来自主题函数；浅色主题下琥珀色和青色会自动加深；角标的描边颜色和它所在的背景一致。
- 截图放进 `research/appearance-check/`，并在本文件末尾记录发现的问题。问题交给 `tree-polish` 任务修复；如果那个任务已经完成，就由本任务直接修。

## Acceptance Criteria

- [ ] 上面 6 组测试全部加上，并且在 Linux CI 上通过。
- [ ] 三种外观的截图已经存档，发现的问题都已修复，或者已经写明原因暂不修。
- [ ] 没有为了让测试通过而放宽任何断言。
