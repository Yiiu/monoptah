# Design: Tree 侧边栏密度与可读性

需求见 `prd.md`（R1–R9），代码事实见 `research/`。行号都以 origin/main 7106f9c1 为准，路径在 `crates/monocode-gpui/src/` 下。设计稿：artboard `tree-sidebar-density`，以「定稿 · 平常」「定稿 · 有待处理 + 悬停」为准。

## 边界

- 只改 Tree 布局。Classic 有自己的导航和项目列表（`project_rail.rs:495`、`:509-575`），不碰。Focused 和 Tree 共用 `sidebar_navigation`、`rail_impl`、`shell_task_chip`、`session_archive_button`，所以 Tree 专属的改动都放在已有的 `tree` 分支或 Tree 的调用点里。
- 不新增设置项，也不新增持久化字段。Sidebar options 的开关、`project_rail_pinned`、`workspace.projects` 的顺序都沿用现有数据。
- 颜色只用 `theme.rs` 的 token（`content`、`accent_color`、`warning_color`、`floating_fill`）。暗色玻璃、暗色纯色、亮色三种外观都要对。
- 后台 git 读取遵循 `state-and-async.md` 的 Background work 模式：放到 background executor，结果回 UI 线程，旧结果要丢弃。

## R1 导航图标条 + Sidebar options 入口

在 `sidebar_navigation`（`shell/sidebar_list.rs:647-903`）的 `tree` 分支里：

- Search 行保持不变，高度从 32 改为 34。peek 时的固定按钮仍放在 Search 行末尾。Search 隐藏时，它和 ⋯ 一起放在图标条行末尾。
- 原来逐行渲染的列表（:765-901），在 Tree 下改成一行 `nav strip`。
  - 过滤规则照旧（:775-786）：New 一直显示，其余项由 `sidebar_*_visible` 和 `preferences.notes` 决定。
  - 每项是一个 28px 的 `icon_button`（`theme.rs:585`），按钮之间间隔 4。图标沿用现有的 glyph。
  - 每个按钮挂一个 `CloudTip` 提示。New 和 Search 的提示是「名称 · 快捷键」，快捷键取自 `settings::command_shortcut`；其余项没有快捷键，只显示名称。
  - 选中态和现在一样：New 取 `blank_session`，其余取 `app_view_label`，选中时用 `content(0.1)` 填充。
  - 角标：Today 在数量大于 0 时显示 6px 的 accent 点，数量写进 aria_label 和提示里。Notes 在数量大于 0 时显示点，`Some(0)` 和 None 都不显示。Inbox 在有未读时显示点。点画在按钮右上角，和设计稿一致。
  - 点击、悬停（`sidebar_hover`）和 debug selector 沿用原来的。selector 保持 `sidebar-navigation-{label}`，测试和快捷入口不用改名。
  - 行末是 ⋯ 按钮（`sidebar-options`）。它的写法照搬 Focused 的开关（`shell.rs:7924-7950`）：用 `aria_expanded`，挂 `sidebar_options_anchor.probe()`，点击在 `open_menu(MenuTarget::SidebarOptions, Anchor::Slot(..))` 和 `close_menu` 之间切换。Focused 和 Tree 不会同时渲染（`shell.rs:7791`），所以同一个 anchor 和 selector 可以复用。
- Focused 的渲染不变。

## R2 Needs you 为 0 时隐藏

- `tree_request_queue` 的空状态（`project_tree/render.rs:797-808`）改成高度为 0 的元素，不再画 "Nothing needs you"。
- 从有到无、从无到有，都走 tree-polish R1 已有的高度 disclosure 动画。Reduce motion 时直接切换。
- 有请求时，每条的内边距调到总高约 44（现在是 `py_1`）。
- Ctrl+J 不变：列表为空时它本来就什么也不做（`project_tree.rs:1222-1232`）。

## R3 Monos 一行头像

- 在 `rail_impl` 的 `tree` 分支（`monos/mod.rs:1585`）里，把多行名册改成一行，高 32：
  - 左边是 "MONOS" 标签，沿用 `tree_section_heading` 的样式，只是不独占一行。
  - 中间每个 Mono 一个 22px 的头像（`monos/avatar.rs:46`）。按 `mono_status()` 画右下角的状态点：working 用 accent，needs-you 用 warning，idle 不画。
  - 每个头像挂 `CloudTip`，内容是「名称 · 状态」。点击调用 `Monos::open`（`:1398`）。
  - 行末是 "+"，复用现在的新建 Mono 入口。
- 没有 Mono 时，这一行只有标签和 "+"。`preferences.monos` 关闭时整行不渲染。`TreeRow::Monos` 仍是列表的第一项。
- Focused 和原导航的分支不变。

## R4 Pin 并入 PROJECTS

- `tree_project_entries`（`shell/project_tree.rs:671-704`）的顺序改为：
  1. `Heading("Projects")`
  2. Pin 的项目，按 `workspace.projects` 的顺序
  3. 每个分组：分组头，再接该组里没 Pin 的项目
  4. 其余没 Pin、也没分组的项目

  这和 `workspace.visible_project_order()`（`workspace.rs:848-876`）的非 Focused 顺序一致，Project: Previous/Next（`step_tree_project`）不用改。
- 删掉 `Heading("Pinned")`。`tree_heading`（`render.rs:331-348`）只给 "Projects" 加按钮，这一点不变。
- `tree_project_row` 的行尾（`render.rs:513-547`）：项目 Pin 了，就在数量和悬停按钮之前画一个 12px 的 `pin` 图标（`content(0.35)`）。悬停时它让位给 "+" 和 "…"。
- 拖拽规则不变：`project_rail_peers` 规定 Pin 的项目只能和 Pin 的项目互换位置。Pin 的最后一个项目块，会在下一个分组头或项目头处结束（`ends_block`）。
- `toggle_project_rail_pin`（`project_rail.rs:1617-1623`）在原有调用之外再调一次 `notify_sidebar`，保证 Tree 立即重绘。
- 每个分组块之间的间距，沿用 R8 的项目间距。

## R5 已读完成的会话行 + 预览里的完成时间

- `tree_session_row` 的行尾 match（`render.rs:948-968`）：
  - `Done` 且已读（`look == 0`）：行尾不放东西。
  - `Done` 且未读：行尾画一个 6px 的 accent 点（`tree_dot`），取代原来的说明文字。行下面的摘要行和头像角标不变。
  - 其他状态的分支不变。
- 悬停时换成归档按钮（`:1044-1048`），这一点不变。
- 预览浮层里的完成时间：
  - 未加载的会话：`session_preview.rs:23-29` 已经在用 `Done.at`。只改说明文字：Done 时显示 `Finished {relative} ago`。
  - 已加载的会话：`Conversation::preview(status)`（`conversation.rs:519-635`）的头部说明，在 `status.activity` 是 `Done { at, .. }` 时改用 `at`，文字同上。其他状态仍按 `status.label(now)` 或 `summary.updated_at`。
  - 相对时间用 `monocode_core::time::relative`，格式不变。两处用同一个小函数生成文字，避免写成两份。

## R6 工作树标题取当前分支

数据来源：一个 Tree 专用的小缓存，写法照抄 `project_tree/tree_diffs.rs`。

- 新建 `shell/project_tree/tree_worktrees.rs`，结构是 `TreeWorktrees { values: HashMap<String /*project path_key*/, (Instant, Vec<git::Worktree>)>, pending: HashSet<String> }`。
- `queue_tree_worktrees(project)` 在渲染项目的工作树行时调用：
  - 数据过期（TTL 20s）或从来没有时，先用 `git_cache::get(project)` 里的 `snapshot.worktrees` 立刻填上，然后在后台跑 `git::worktrees(project)`（`monocode-core/src/git.rs:233`，比 `snapshot` 轻），回来后写入缓存并 `notify_sidebar`。
  - `cfg!(test)` 下不发起后台任务，测试直接往缓存里写数据。
  - 窗口重新获得焦点（`observe_window_activation`）时，把所有项目标记为过期，这样在别处切了分支，回来就能看到。
- 标题解析是 `tree_worktree_title(project, path) -> (title, detached)`，按 `path_key` 找到对应的 `Worktree`：
  - 有 `branch`：标题就是分支名。主检出也一样，例如 `main`。
  - 没有 `branch`（detached）：标题是目录名（`project_name(path)`），`detached = true`。
  - 缓存里没有这个工作树（还没加载完，或者是远程路径）：先用 `tree_branch(id)` 兜底，最后才用目录名。
- `TreeRow::Worktree.branch` 改为在构建行时就放进解析好的标题和 `detached`。`replace_rows` 按 `PartialEq` 比较，所以标题一变就会重绘。
- `tree_worktree_row`（`render.rs:721-778`）：
  - detached 时，在标题后面追加 `· detached` 说明（`tree_caption`，`content(0.35)`）。
  - 整行挂 `ProjectTooltip`，显示完整路径。
  - 悬停 "+" 用的 `WorktreeFocus.branch` 改用解析出的分支，detached 时为 None。
- `focus_tree_session_worktree`（`project_tree.rs:1177-1192`）同样改用解析出的分支。
- 不改 `sessions.branch` 的写入。这是另一个问题，不在本任务里。

## R8 间距

- 会话行：`content_row` 从 24 改为 26（`render.rs:1014` 起）。`rails::step` 上下各 2 的间隔不变，tree-polish 约定轨道线连续要靠它。
- 项目行：除了紧跟在标题或分组头后面的第一个项目，其余项目行的外层 wrapper（`render.rs:324-328`）加 `pt(6)`。分组头也一样。
- 区块标题：`tree_section_heading` 上方留 12。
- 导航按钮 28、间距 4，Search 34，Monos 行 32，Needs you 每条 44：都在 R1–R3 的对应位置实现。
- 所有尺寸写成具名常量，放在各自模块的顶部。

## R9 轨道线不被高亮盖住

- 会话行的选中、悬停、拖放填充现在画在 `content_row` 上（`render.rs:1096-1101`），而 `content_row` 在 `rails::step` 的左内边距（20）之内，所以轨道线本来就可见。本任务改行高时，要保证填充仍然只画在 `content_row` 上，不能挪到外层的 step wrapper。
- 工作树行（`tree_node`）的悬停填充也要从轨道线右侧开始。如果现在是整宽填充，就按会话行的左边距收窄。
- 加测试锁定：选中会话行的填充左边界大于轨道线的 x。

## 测试

全部用 `crate::shell::tests::window(cx)` 和 `VisualTestContext`，路径用 `/work/...`，快捷键用 `secondary-`，在 Linux 上也能跑。

| 需求 | 测试 |
|---|---|
| R1 | 导航图标条是一行：各按钮的 top 相同，按钮 28px。⋯ 能打开 Sidebar options；关掉 Inbox 后对应按钮消失，切到 Focused 后仍是关闭状态。Notes 为 `Some(0)` 时没有角标。更新 `layout.rs:977` 和 `:1034` 这两个现有测试（它们断言的是整行导航）。 |
| R2 | Needs you 为 0 时，没有 `tree-needs-you-*` 行，第一个列表行紧接在导航下面；加一条请求后它出现。 |
| R3 | Monos 只占一行：所有头像的 top 相同；点头像打开对应的 Mono。更新 `project_tree.rs:1401`（它断言的是 Monos 标题高 26）。 |
| R4 | Pin 一个分组里的项目后，它排在 PROJECTS 下面、所有分组之前；取消 Pin 后它回到原分组；整个过程中没有 "Pinned" 标题。 |
| R5 | 已读完成的行没有 `tree-session-trailing-*`，悬停时出现 `archive-session-*`。预览浮层的说明文字是 "Finished …"，已加载和未加载的会话各测一条。 |
| R6 | 往缓存里写入两个工作树：`/work/app` 在 `main`，`/work/app-wt` 在 `feat/x`。会话没有记录分支时，节点标题是 `main` 和 `feat/x`。改成 detached 后显示目录名和 "detached"。缓存刷新后标题跟着变。 |
| R8/R9 | 会话行高 30。选中行填充的左边界大于轨道线的 x。 |

## 兼容与回退

- 没有数据迁移，也没有新增持久化字段，回退就是 revert 提交。
- 唯一的新后台任务是 R6 的 `git worktree list`：每个项目最多每 20 秒一次，只在 Tree 布局渲染到该项目的工作树行时才触发。
