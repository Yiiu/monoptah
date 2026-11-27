# Implement: Tree 侧边栏密度与可读性

依据：`prd.md`（R1–R9）、`design.md`、`research/`。按 CLAUDE.md「Developing the native app」执行：一个任务的改动全部写完后，只跑一轮检查。

## 0. 准备

- [ ] 从 `origin/main` 新建工作树和分支 `feat/tree-density`。本地 `main` 落后，不要在主检出上改。

  ```bash
  git fetch origin
  git worktree add .worktrees/tree-density -b feat/tree-density origin/main
  ```

- [ ] 在跑第一条 cargo 命令之前，先复制构建缓存（Windows 上用普通 `cp -R`）：

  ```bash
  cp -R crates/monocode-gpui/target .worktrees/tree-density/crates/monocode-gpui/target
  ```

- [ ] 在新的工作树里登记分支：`python ./.trellis/scripts/task.py set-branch feat/tree-density`。
- [ ] `11-27-tree-density` 和它的父任务目录在 origin/main 上不存在（只在本地是未跟踪文件）。把整个任务目录复制进新工作树，和代码一起提交。

## 1. 实现（顺序即依赖）

1. [ ] **R8 常量与行高**：在 `render.rs` 增加具名常量；`content_row` 24→26；项目行和分组头的外层 wrapper 加 `pt(6)`（每段第一个除外）；`tree_section_heading` 上方留 12。
2. [ ] **R9**：确认会话行的填充仍画在 `content_row` 上；把 `tree_node` 的悬停填充左边界对齐到轨道线右侧。
3. [ ] **R2**：在 `tree_request_queue` 里去掉空状态；每条请求的内边距调到总高 44。
4. [ ] **R1**：在 `sidebar_navigation` 的 `tree` 分支实现导航图标条和 ⋯ 按钮（照搬 `shell.rs:7924-7950`）；Search 改为 34；Notes 为 0 时不显示角标；New 和 Search 的提示用 `command_shortcut` 生成。
5. [ ] **R3**：在 `rail_impl` 的 `tree` 分支实现一行头像：状态点、提示、点击打开、"+"。
6. [ ] **R4**：调整 `tree_project_entries` 的顺序；在 `tree_project_row` 里加 pin 标记；`toggle_project_rail_pin` 增加 `notify_sidebar`。
7. [ ] **R5**：`Done` 已读时行尾不放东西，未读时画点；抽出一个生成 "Finished …" 文字的小函数，在 `session_preview.rs` 和 `Conversation::preview` 里都用它。
8. [ ] **R6**：新建 `tree_worktrees.rs` 缓存（先用 `git_cache::get` 立刻填上，再在后台跑 `git::worktrees`，TTL 20s，窗口重新获得焦点时标记过期，`cfg!(test)` 下不发起后台任务）；实现 `tree_worktree_title`；改 `TreeRow::Worktree` 的标题字段和 `tree_worktree_row`（detached 说明、路径提示、`WorktreeFocus.branch`）；同步改 `focus_tree_session_worktree`。
9. [ ] 按 `design.md` 的测试表补上新测试，并更新现有的 `layout.rs:977`、`layout.rs:1034`、`project_tree.rs:1401`。

## 2. 验证（只跑一轮）

都在 `crates/monocode-gpui` 目录下用 native-dev 别名执行：

```bash
cargo fmt
cargo nclippy -- -D warnings
cargo ntest -- shell::project_tree shell::sidebar_list shell::layout monos shell::rails session_preview conversation::preview
```

- 只修失败的，再只重跑失败的测试。
- 改到了 `conversation.rs`，所以 core 和 GPUI 的完整检查交给 CI（`native.yml`）。本地不跑 `npm run native:check`。
- Windows 上读退出码不要经过管道：`cargo ntest ... > log 2>&1; echo $?`。

## 3. 用眼睛确认

- [ ] 从这个工作树启动 Native Dev：`npm run native:dev -- --detach`。切到 Tree 布局，在三种外观（暗色玻璃、暗色纯色、亮色）下各截一张图，和设计稿对照。截图放进 artboard 的 verify 组。
- [ ] 在 monoptah 项目里实际确认：工作树节点显示分支名，不再是项目名；在工作树里切一次分支，切回窗口后标题跟着变。
- [ ] 确认 Classic 和 Focused 没有变化：各截一张图，和改动前对比。

## 4. 评审

- [ ] 交给评审代理一份清单：PRD 每条需求对应改了哪里（`file:line`）、对应哪条测试。评审只看代码，只在需要证明某个发现时才构建或跑测试。

## 5. 收尾

- [ ] 提交：`feat(native): make the Tree sidebar denser and show worktree branches`，正文写明 R1–R9 的原因。PR 按模板填写。
- [ ] 合并后按 `cleanup-worktree` 技能清理这个工作树。

## 回退点

- 第 1–3 步只改布局，第 4–8 步各自独立。任何一步出问题，单独 revert 那一部分即可，没有数据需要回滚。
