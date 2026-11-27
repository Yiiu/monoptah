# Tree 侧边栏密度与可读性

## Goal

Tree 布局的侧边栏顶部太高：在 905px 高的窗口里，PROJECTS 从 y≈555 才开始，只能看到 4 个项目和 8 个会话。会话标题也被行尾的状态列挤掉。本任务把高度和宽度还给项目与会话行，并让 Tree 布局能打开 Sidebar options 菜单。

设计稿：artboard `tree-sidebar-density`（`.artideck/artifacts/artboard/tree-sidebar-density/`）。

## Requirements

已确认（2026-11-27）：

- R1 顶部导航（Notes 数量为 0 时不显示角标）：Search 保持一行输入框。Today、New、Inbox、Notes、Automations、Contributors 合成一行图标条，悬停时显示名称和快捷键。Notes 有内容时显示角标。行尾放一个 ⋯ 按钮，打开现有的 Sidebar options 菜单（`sidebar_options_menu_items`），在 Tree 里也能逐项隐藏。
- R2 Needs you：数量为 0 时整段隐藏，不再显示 "Nothing needs you"。Ctrl+J 不变。有请求时保持现有的列表和数量显示。
- R3 Monos：名册压成一行。依次是 "MONOS" 标签、每个 Mono 一个头像（悬停显示名字和状态，运行中带状态点）、行尾 "+"。关闭 Monos 偏好时整行隐藏。
- R4 Pin：取消单独的 PINNED 分区。PROJECTS 标题（带筛选和 "+"）挪到项目区最上面，下面依次是 Pin 的项目、各个分组、其余项目。Pin 的项目行尾显示 pin 标记。拖拽排序和 Pin 数据仍与 Classic 共用。在 Tree 里 Pin 或取消 Pin 后列表立即重绘。
- R5 会话行状态列：已完成且已读的会话不显示状态列（勾选图标和相对时间）。悬停时行尾照旧显示归档按钮。完成时间不另加提示，放在现有的会话预览浮层里：浮层对已完成的会话显示完成时间（例如 "Finished 1h 19m ago"），已加载的会话也一样，不再显示最后更新时间。运行中显示当前步骤，需要你处理显示 Needs you，未读显示一个点。
- R7 终端数标记 "⊡N"：不改。它本来就只统计 agent 上报、仍在运行的后台任务，任务结束、回合结束或停止后消失（Q10，2026-11-27）。
- R8 间距（设计稿定稿，2026-11-27）：
  - 会话行总高从 28 改为 30：内容行从 24 改为 26，上下各 2 的间隔保持不变（tree-polish 任务约定轨道线连续要靠它）。项目和项目之间多留 6px。区块标题上方留 12px。
  - 导航图标按钮 28px，间距 4px。Monos 那一行 32px。Search 框 34px。Needs you 每条 44px。
- R9 树的轨道线：会话行和工作树行的选中、悬停、拖放高亮都从轨道线右侧开始，轨道线始终连续可见，与现状一致。不能有任何高亮盖住轨道线。

- R6 工作树：保留现有的工作树节点行（设计稿第 2 轮选 A）。
  - 问题：用户看到节点显示的是项目名，而不是工作树。节点标题现在是 `branch.unwrap_or_else(|| project_name(&path))`（`project_tree/render.rs` 的 `tree_worktree_row`）。`branch` 取自组内第一个会话：已加载的会话读它的 `branch` 字段，没加载的读 `session_details.branch`。本机数据库 `sessions.branch` 和 `worktree_cwd` 全部为空（2026-11-27 只读查询，41 个未归档会话）。所以多数情况下 `branch` 是 None，标题退化成路径的最后一段，主检出那一行就成了项目名（例如 `monoptah`）。
  - 要求：节点标题取该工作树**当前**检出的分支，直接从 git 的工作树列表读（`git::snapshot(..).worktrees` 带路径和分支），不再依赖会话记录。主检出也显示分支名（例如 `main`）。切换分支或改名后，在下次刷新 git 状态时更新。
  - 读不到分支时（detached HEAD、git 失败）：显示工作树目录名，后面加一个灰色的 "detached"。悬停标题时显示完整路径。

## Constraints

- 沿用父任务 handoff 的约束：不新增设置项，颜色只通过 `theme.rs` 取，暗色玻璃、暗色纯色、亮色三种外观都要正确，Classic 和 Focused 布局不能变。
- Sidebar options 的开关状态与 Focused 共用同一份 workspace 字段。

## Acceptance Criteria

- [ ] 默认设置、905px 高的窗口下，PROJECTS 标题出现在侧边栏顶部 200px 以内。
- [ ] 在 Tree 布局里能打开 Sidebar options 并隐藏任一导航项，切到 Focused 后状态一致，重启后保留。
- [ ] Needs you 为 0 时侧边栏里没有对应的行；有请求时出现，Ctrl+J 能跳到第一条。
- [ ] Monos 只占一行；悬停头像能看到名字；点击头像打开该 Mono。
- [ ] 不再出现 PINNED 标题；PROJECTS 标题在最上面，Pin 的项目排在分组之前；Pin 和取消 Pin 后列表立即按新顺序重绘。
- [ ] 已读完成的会话行不显示时间；悬停时出现归档按钮。会话预览浮层显示完成时间，已加载和未加载的会话都是。运行中、需要你处理、未读这三种状态照常显示。
- [ ] 工作树节点标题是该工作树当前的分支名，主检出也是（例如 `main`），不再出现项目名。会话没有记录分支时也一样。
- [ ] 工作树切换分支或分支改名后，Tree 里的节点标题显示新分支名，不需要重启。
- [ ] detached HEAD 的工作树显示 "目录名 · detached"，悬停能看到完整路径。
- [ ] 选中和悬停会话行时，左侧的轨道线仍然完整可见。
- [ ] Classic 和 Focused 布局的截图与改动前一致。

## Out of scope

- 切换窗口后整个窗口变白：`11-27-window-unfocus-white`。
- 收起后的 48px 状态栏：`11-27-tree-collapsed-bar`。

## Notes

- 事实来源：origin/main `shell/project_tree/render.rs`、`shell/project_tree.rs`、`shell/sidebar_list.rs`、`monos/mod.rs`。
- 侧边栏默认宽 260，可拖动范围 200–560。
