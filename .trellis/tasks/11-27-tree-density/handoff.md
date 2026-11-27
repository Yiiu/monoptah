# Handoff: 11-27-tree-density

接手的 agent 从这里开始读。需求和设计都已经和用户逐条确认过（2026-11-27），不要再重新讨论这些决定；如果实现中发现某条做不了，先停下来问用户。

## 阅读顺序

1. `prd.md`：需求 R1–R9 和验收标准，这是唯一的需求来源。
2. `design.md`：每条需求改哪里、怎么改，以及测试表。
3. `implement.md`：步骤、验证命令和评审方式。
4. `research/`：origin/main 7106f9c1 上的代码事实（file:line）。行号如果对不上，以当时的代码为准。
5. `mockups/final-vs-current.png`：左边是现状，右边两张是定稿。像素以这张图为准。
   - `mockups/html/`：设计稿源码。图标是画板的自定义元素，直接用浏览器打开时不会显示。
   - `mockups/worktree-options.png`：工作树三种方案的对比，用户选了 A（保留节点行）。

## 已确认的决定（摘要）

| 问题 | 决定 |
|---|---|
| 顶部导航 | Search 保持一行；其余入口合成一行 28px 图标；⋯ 打开 Sidebar options（Tree 里原来没有入口） |
| Needs you 为 0 | 整段隐藏 |
| Monos | 一行：标签 + 头像（带状态点和名称提示）+ "+" |
| Pin | 不要 PINNED 分区；PROJECTS 标题在最上面，Pin 的项目排在分组前 |
| 已读完成的会话 | 行尾不放东西；悬停仍是归档按钮；完成时间放进现有的会话预览浮层（"Finished … ago"），不新增提示 |
| ⊡N 终端数 | 不改 |
| 工作树 | 保留节点行；标题改为工作树当前的分支（直接从 git 读）；detached 显示「目录名 · detached」，悬停显示完整路径 |
| 间距 | 会话行高 30（内容 24→26，上下各 2 的间隔不动），项目之间 +6，区块标题上方 12 |
| 轨道线 | 任何高亮都不能盖住会话行左侧的树轨道线 |

工作树标题的问题：用户看到节点显示的是项目名。原因是会话的 `sessions.branch` 在本机数据库里全为空（只读查询，41 个会话），标题就退回成了路径的最后一段。

## 不在本任务里

- 切窗口后整窗变白：`11-27-window-unfocus-white`
- 收起后的 48px 状态栏：`11-27-tree-collapsed-bar`

## 开工前要注意

- 本地主检出 `C:\dev\monoptah` 的 main 已经和 origin/main 对齐（7106f9c1）。但工作区里有一处**不属于本任务**的、未提交的 `crates/monocode-core/src/provider.rs` 改动，不要动它，也不要把它带进你的分支。
- 本任务目录只存在于主检出的工作区里（未跟踪），origin/main 上没有。新建工作树后，把整个 `.trellis/tasks/11-27-tree-density/` 复制过去，和代码一起提交。父任务 `11-25-agent-overview/task.json` 的 children 里加了本任务和两个兄弟任务（主检出里未提交的改动），PR 里一起带上。
- 开工：在新工作树里运行 `python ./.trellis/scripts/task.py start .trellis/tasks/11-27-tree-density`，然后按 `implement.md` 执行。
- 构建缓存：先从主检出复制 `crates/monocode-gpui/target`，再跑 cargo。只用 native-dev 别名（`cargo ntest` / `nclippy`），整个任务只跑一轮测试。见 CLAUDE.md「Developing the native app」。
- 测试要能在 Linux 上跑：快捷键用 `secondary-`，路径用 `/work/...`。CI 在 Linux 和 Windows 上跑 GPUI。
- 本机的画板服务是 Artideck 1.108.7（为绕开上游 bug #56 退回的），设计稿目录 `.artideck/`、`.ryugi/` 是联接。和本任务的代码无关，不要提交它们。
