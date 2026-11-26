# Tree 布局打磨：样式与缺失入口

Parent: `11-25-agent-overview`。开工前先读父任务的 `handoff.md`。本任务的问题来源于审计
`11-25-agent-overview/research/audit-tree-summary.md`，文中写的"第 N 项"和"(a)–(g)"都指这份审计里的编号。

## Goal

把 Tree 布局里剩下的样式问题和功能入口补齐，让它和定稿设计稿一致，同时不影响 Classic 布局。

## Requirements

### R1 「Needs you」数量为 0 时不再显示琥珀色（f）
- 树顶部的队列（`shell/project_tree/render.rs:601,618`）：数量为 0 时收成一行暗色文字 "Nothing needs you"，使用 `content(0.4)` 这一级的颜色，不要背景填充，也不要琥珀色。
- 收起状态栏里的 "✋ 0"（`render.rs:989`）：数量为 0 时用中性色，大于 0 时才用 `warning_color()`。
- 数量从 0 变成 N 时，展开有高度动画；开启"减少动态效果"时直接切换。

### R2 区块标题统一（g）
- Tree 布局下，MONOS、PINNED、分组名、PROJECTS 这些区块标题使用同一种样式：现有树标题的大写小号字（`render.rs:250-260`）。
- Mono 名册的标题在 Tree 布局里改用这个样式，Focused 布局里保持原样。

### R3 分组头的 Tree 样式和展开动画（第 4 项，e）
- 分组头在 Tree 布局下的样式和其他树节点对齐：行高 30px、悬停 `content(0.05)`、选中 `content(0.10)`、圆角 6px。
- 展开和收起走 `tree_disclose`，用高度动画，箭头同步旋转，和项目节点一致。
- Classic 项目栏里分组头的样式和行为一点都不能变：Tree 走单独的渲染分支，不改共用代码路径上的数值。

### R4 只有一个 worktree 时不显示 worktree 层（第 2 项）
- 判断是否显示 worktree 层时，只统计不在 Pinned、Reminders 或文件夹里的会话（`shell/project_tree.rs:547-554` 要和 `:940-948` 用同一套口径）。
- 例子：一个项目只有一个被 Pin 住的主目录会话。此时不应出现单独一行 "main" worktree。

### R5 Tree 里能找到「New session in Codespace…」（第 1 项）
- 项目右键菜单和「⋯」菜单里，在 `offers_codespace()` 为真时加上这一项，行为与 Focused 顶部菜单里的那一项（`shell.rs:6491-6500`）完全相同。

### R6 worktree 行的「+」（第 5 项）
- 鼠标悬停在 worktree 行上时，显示和项目行一样的「+」按钮，点击后在这个 worktree 里新建会话。如果这个 worktree 里已经有一个还没动过的空白会话，就直接切过去，不再新建。
- 用户已确认要做（2026-11-26）。

### R7 滑出面板里角标的描边颜色（第 7 项）
- 树从左边滑出（peek）时，背景是 `floating_fill()`。行内状态角标的描边颜色要跟着这个背景走，不能继续用 `shell_fill()`。

### 不做的项
- (d) 项目收起或展开时计数淡入淡出：不做（用户已于 2026-11-26 确认）。定稿设计稿里，展开的项目也一直显示计数，计数用来做汇总。
- 第 6 项会话行间距为 4px：保留。竖线必须穿过连线组件自带的内边距才能连续（见规范 `component-guidelines.md` 的 "Tree rails"）。

## Acceptance Criteria

- [ ] 「Needs you」为 0 时，树和收起状态栏里都没有琥珀色（测试检查颜色或渲染状态）。
- [ ] Tree 布局下所有区块标题样式一致；Focused 和 Classic 下的 Mono 名册标题不变（测试）。
- [ ] Tree 里分组头的展开和收起会经过 disclosure 动画；Classic 下分组头的尺寸和样式不变（两个布局各有一个测试）。
- [ ] 一个只有被 Pin 会话的单目录项目，不渲染 worktree 行（测试）。
- [ ] 在支持 Codespace 的环境下，Tree 的项目菜单里有「New session in Codespace…」（测试）。
- [ ] worktree 行的「+」会在对应的 worktree 里新建会话，已有空白会话时直接复用（测试）。
- [ ] 深色玻璃、深色不透明、浅色三种外观下，peek 面板里的角标描边都和背景一致（实机截图）。
