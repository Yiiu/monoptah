# Tree 侧边栏评审清单

分支 `feat/tree-density`，基线 `origin/main` 的 `7106f9c1`。以下源码路径相对于 `crates/monocode-gpui/src/`。

| 需求 | 实现入口 | 回归覆盖 |
| --- | --- | --- |
| R1 导航图标、角标和 Sidebar options | `shell/sidebar_list.rs:654` 共用菜单入口，`:674` 渲染 Tree 搜索与图标条。 | `tree_navigation_is_a_strip_with_shared_sidebar_options`、`tree_sidebar_keeps_the_content_gutter_while_closing`、`tree_shortcuts_hide_the_sidebar_and_allow_peek` |
| R2 Needs you 空态与过渡 | `shell/project_tree/render.rs:20` 更新队列和过渡，`:850` 渲染该区块。下一请求沿用 `shell/project_tree.rs` 的 `open_next_tree_request`。 | `needs_you_collapses_when_empty_and_shows_44_pixel_requests`、`tree_request_disclosure_animates_open_and_closed`、`needs_you_opens_the_other_project_and_uses_its_worktree` |
| R3 Monos 单行 | `monos/mod.rs:1860` 渲染头像、状态点、提示与新建入口。 | `tree_mono_avatars_share_a_row_and_open_the_clicked_mono`、`only_the_tree_layout_uses_the_tree_mono_heading` |
| R4 项目 Pin | `shell/project_tree.rs:700` 构建项目顺序，`shell/project_tree/render.rs:504` 绘制项目行和 pin，`shell/project_rail.rs:1623` 刷新 Tree。 | `tree_pins_move_above_groups_without_absorbing_ungrouped_projects`、原有 Tree 分组与项目拖拽测试 |
| R5 完成状态与预览 | `shell/project_tree/render.rs:955` 渲染会话行，`shell.rs:7954` 挂载悬停预览，`shell/session_preview.rs:112` 与 `conversation.rs:519` 共用 `session_mark.rs:76` 的文案。 | `read_finished_tree_rows_clear_the_status_and_keep_hover_and_rail`、`unopened_tree_preview_uses_the_receipt_even_when_the_session_was_updated_later`、`loaded_tree_preview_uses_the_receipt_instead_of_the_last_update` |
| R6 工作树分支、detached 和刷新 | `shell/project_tree/tree_worktrees.rs:130` 发起后台 Git 查询，`shell/project_tree.rs:1212` 解析标题与分支，`shell/project_tree/render.rs:767` 渲染节点和路径提示，`shell.rs:1258` 与 `:9887` 处理 Git 状态刷新和窗口重新聚焦。 | `tree_worktree_rows_follow_the_live_branch_and_detached_state` 覆盖切换悬停时的完整路径、分支名和 detached，`worktree_hover_button_creates_and_reuses_a_blank_in_that_copy` 覆盖 `+` 按钮。 |
| R7 终端数 | `shell/project_tree/render.rs` 的 `tree_session_row` 沿用 `shell/rails.rs:782`。 | `classic_and_focused_show_a_chip_for_background_shells` |
| R8 密度与间距 | `shell/project_tree/render.rs:4` 行高与间距常量，`:1218` 项目间距，`:366` 项目标题。导航、Monos 和请求行的尺寸在各自的实现中。 | `tree_navigation_is_a_strip_with_shared_sidebar_options`、`needs_you_collapses_when_empty_and_shows_44_pixel_requests`、`tree_pins_move_above_groups_without_absorbing_ungrouped_projects`、`read_finished_tree_rows_clear_the_status_and_keep_hover_and_rail` |
| R9 轨道线 | `shell/project_tree/render.rs` 的 `tree_row` 和 `tree_session_row` 把会话高亮放在 `shell/rails.rs:72` 的留白内，`:767` 将工作树高亮缩进。 | `read_finished_tree_rows_clear_the_status_and_keep_hover_and_rail`、原有 Tree 对齐与拖拽测试 |

评审时重点确认 Git 查询期间重新聚焦是否仍会触发刷新、项目分组的拖拽边界、窄侧边栏中的菜单可达性，以及完成时间文案是否只在 Tree 显示。

## 本地验证记录

- Windows Native Dev 已构建，`cargo fmt --check` 已通过。工作树悬停的单项 GPUI 测试已通过，依次悬停两行时显示各自的路径。
- 本地筛选测试匹配了 845 项，运行在 Mono 测试中异常退出，分片日志也没有完整结果。Windows 严格 Clippy 仍报出本分支未改动的 `automations/editor.rs`、`terminal.rs` 和 `today/note_composer.rs` 中的警告。完整检查待 `native.yml` 的 Linux 和 Windows CI。

## Windows Native Dev 实机验收

2026-11-27 17:48（UTC+08:00），使用 `feat/tree-density` 的 `native-dev` 构建，程序 SHA-256 为 `CCE42D5E3C869CE53254EF9688CD45219238D645BCE2D8EF9684A38AA4965CDF`。会话数据放在独立的 Dev 数据库中，工作树路径指向真实的 Git 检出。截图存于本机 `D:/monoptah-build-cache/tree-density/verification/`。

| 检查 | 实机观察 | 截图文件 |
| --- | --- | --- |
| R6 实际工作树悬停 | `main` 显示 `C:/dev/monoptah`，`feat/tree-density` 显示 `C:/dev/monoptah/.worktrees/tree-density`。无障碍节点包含完整分支名。 | `real-main-hover.jpg`、`real-tree-density-hover.jpg` |
| R6 切换 Git 分支后刷新 | 隔离检出从 `feat/sidebar-qa` 切到 detached HEAD，再切回原分支。两次重新聚焦都刷新了标题。 | `worktree-refresh-before.jpg`、`worktree-refresh-detached.jpg`、`worktree-refresh-restored.jpg` |
| Tree 平常视图与 Sidebar options | 对照定稿核对了单行导航、Monos 头像条、空 Needs you 收起、工作树层级和轨道线。暗色玻璃、暗色纯色、亮色均显示上述结构。实际点击 `⋯` 打开了侧栏选项。 | `tree-dark-glass.jpg`、`tree-dark-solid.jpg`、`tree-light.jpg`、`tree-sidebar-options.jpg` |
| Classic、Focused | 与改动前的截图比对了导航、项目和会话区的结构与位置。截图的玻璃背景、打开的标签和相对时间存在差异。 | `classic-before.jpg`、`classic-after.jpg`、`focused-before.jpg`、`focused-after.jpg` |

有待处理请求的画面和已完成会话的悬停预览仍由回归测试及评审检查。本轮三种外观截图使用的是空请求状态。
