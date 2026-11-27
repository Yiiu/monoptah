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
- [x] 三种外观的截图已经存档，发现的问题都已修复，或者已经写明原因暂不修。
- [x] 没有为了让测试通过而放宽任何断言。

## Windows 外观核对

2026-11-26 在 Windows 11 上用 Rust 1.98.1 启动 Native Dev。独立的演示数据库包含运行、等你、完成未读和失败四种会话，以及同一工作树内的五个标签页。临时应用内探针通过 Appearance 设置事件切换外观，再调用侧栏收起和 Peek 的界面入口。用 PrintWindow 捕获对应 Native Dev 进程的窗口，每张图为 1336 × 828。

| 外观 | 展开 Tree | 收起状态栏 | 左侧 Peek |
| --- | --- | --- | --- |
| 深色玻璃，侧栏 85%，玻璃正文开启 | [Tree](research/appearance-check/dark-glass-tree.png) | [状态栏](research/appearance-check/dark-glass-status-bar.png) | [Peek](research/appearance-check/dark-glass-peek.png) |
| 深色不透明，侧栏 100%，玻璃正文关闭 | [Tree](research/appearance-check/dark-solid-tree.png) | [状态栏](research/appearance-check/dark-solid-status-bar.png) | [Peek](research/appearance-check/dark-solid-peek.png) |
| 浅色，侧栏 85%，玻璃正文关闭 | [Tree](research/appearance-check/light-tree.png) | [状态栏](research/appearance-check/light-status-bar.png) | [Peek](research/appearance-check/light-peek.png) |

对照定稿核对了四种行状态、收起栏的项目汇总点、Peek 的角标边缘，以及共用的标签页图标。深色玻璃和深色不透明的正文同一位置分别采样为 `#151515` 和 `#0a0a0a`。九张截图的侧栏区域均采到主题定义的琥珀色和青色。浅色从深色的 `#fbbf24`、`#2dd4bf` 切换到 `#b45309`、`#0f766e`。Peek 核对使用了 `floating_fill()`，停靠的 Tree 与收起状态栏使用 `shell_fill()`。角标边缘沿用各自所在表面，所选行沿用该表面与选中底色的混合色。

### 发现与处理

- Focused 的徽标容器漏算后台 shell 小标签。已将独立小标签纳入显示条件，回归覆盖没有其他徽标的会话。
- 第一轮截图带入了主机的工具更新卡片和用量页脚。重新捕获时只调整演示夹具，使用离线的 Hermes 会话，并在夹具中隐藏相应的更新提示。归档截图中的会话和提问均为演示内容。
- 有标题的问题卡在 Tree 队列和行内会显示该卡片的标题。定稿示例使用问题正文。这次保留共享的通知摘要规则，避免把多问题卡片任意压缩成其中一个问题。

### 本地检查

Windows 的格式检查通过。Core 的 110 项原有测试通过，补齐假 ACP 会话的模式设置握手后，Hermes 回归定向重跑通过。GPUI 首轮 143 项通过。修正 Focused 徽标容器和 Mono 测试夹具的侧栏重绘后，两项定向重跑通过。严格 Clippy 报告了交接文档列出的八条原有 Windows 诊断。Linux CI 由 PR #22 验证。
