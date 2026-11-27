# Windows 系统提醒（R6）

Parent: `11-25-agent-overview`。开工前请先读父任务的 `handoff.md`。这一项原本是第 1 期的 R6，当时用户决定暂缓，现在单独立项。

研究和设计都已经做完，直接沿用：
- 研究：`11-25-agent-status-core/research/windows-attention.md`
- 设计：`11-25-agent-status-core/design.md` 第 7 节
- 原执行计划：`11-25-agent-status-core/implement.md` 中的 PR-B。按父任务约定，本项**不单独开 PR**，在本分支完成后合回 PR #22。

## Goal

Windows 上窗口不在前台时，如果有会话需要你处理，你要能注意到。

## Requirements

当某个会话进入 NeedsYou 状态，且窗口不在前台时：

1. **系统通知**：标题写"项目 · 会话"，正文是 agent 的原话，带一个「Open」按钮。点击后进入该会话，这个会话同时算作已读。没有处理的通知保留在通知中心。
2. **任务栏数字角标**：数字是处于 NeedsYou 的会话数，随数量变化更新，降到 0 时清除。
3. **任务栏闪烁**：一直闪，直到窗口回到前台。
4. **提示音**：在 Windows 上也能播放应用现有的提示音。
5. **去重**：同一个会话 5 秒内只弹一次通知。
6. **前台不弹**：窗口在前台、并且正看着那个会话时，不弹通知。
7. **完成不弹**：会话完成只影响角标计数（不变），不弹通知。
8. **开发版和正式版各用独立的应用身份**：AUMID 分别是 `com.monoptah.native` 和 `com.monoptah.native.dev`，两者的通知设置互不干扰。
9. **不新增设置项**：沿用现有的通知开关和静音规则。macOS 的现有行为保持不变。

## Out of scope

- 安装器给快捷方式写入 AUMID。先实测：如果任务栏把固定的快捷方式和运行中的窗口分成两个按钮，再另开任务处理。
- 应用退出之后，从通知中心点击通知重新启动应用（这需要 COM 激活器）。

## Acceptance Criteria

- [x] 可移植的纯逻辑部分都有测试，并且能在 Linux 上运行：tag 解析（`Target::from_tag` 在 Windows 上启用）、5 秒去重、通知标题和正文的格式、角标图片的像素数据（数字字形和 "9+"）。
- [x] Windows 11 实机验证（结果写进 commit 正文或 PR 评论）：
  - 窗口在后台、有会话需要处理时，系统通知、角标、闪烁、提示音都出现；
  - 点「Open」进入对应会话，并且该会话算作已读；
  - 弹窗消失后，从通知中心点击仍然有效；
  - 记录任务栏是否把快捷方式和窗口分成两个按钮；
  - 开发版和正式版的通知设置互不影响。
- [ ] `Cargo.lock` 已更新（CI 用 `--locked` 构建）；Linux 上的 clippy 和 Windows 上的 `cargo check` 都通过。

## 验收记录

2026-11-26 在 Windows 11 Sandbox 26100 上运行提交 `810c3068` 的 Native Dev 构建。两个测试实例使用独立会话库和模拟 Hermes ACP，一份带安装标记，另一份使用 Dev 身份。Sandbox 需要应用目录内的 VC++ 运行库，以及编译时源码路径下的三份 GPUI 调试着色器。

| 要求 | Windows 11 观察结果 |
|---|---|
| 1. Toast、Open 和已读 | 后台收到 `Project · Windows attention fixture: two approvals`，正文为 `Allow the synthetic notification test?`。另一个会话的弹窗 Open 直接切入对应会话。第一条提醒在弹窗消失、会话结束后仍留在通知中心。点展开后的 Open 激活对应会话，SQLite 中 `seen_at` 从空变为该轮的结束时间。 |
| 2. 任务栏角标 | 单个会话内两个待处理请求合计为 1，会话结束后降到 0 并清除。另一次测试中，待处理的会话增加时分别显示 2 和 3。 |
| 3. 任务栏闪烁 | 后台批准请求到达后，运行中的窗口按钮显示橙红闪烁底色。点击弹窗 Open 回到会话后，底色消失，尚待处理的角标仍在。 |
| 4. 提示音 | Windows 音频会话按应用 PID 模拟请求期间测得峰值 0.0530，生成的 `input.wav` 为 192044 字节。 |
| 5. 去重 | 同一会话 1.21 秒内先后要求批准两件事，通知中心保留一条提示。 |
| 6. 前台抑制 | 当前正在查看的会话出现批准请求，待处理角标从 1 更新为 2，系统没有弹窗，通知中心为空。 |
| 7. 完成抑制 | 独立会话在后台完成，系统没有弹窗，通知中心为空，音频峰值为 0。两个其他会话仍在等待，角标留在 2。 |
| 8. 身份与独立设置 | 两个 Windows 发送方分别注册为 `com.monoptah.native.dev` 和 `com.monoptah.native`。在 Windows 通知设置里关掉 Dev 项，正式身份项保持开启。 |
| 9. 现有规则 | 发送路径沿用已有通知、声音与项目静音设置，macOS 后端沿用原实现。 |

任务栏实测已确认未标记 AUMID 的 `Monoptah Fixture.lnk` 出现在固定列表。任务栏显示该快捷方式和另一个带数字角标的运行中窗口按钮。这个测试使用带安装标记的 Native Dev 构建和按现有 NSIS 方式创建的快捷方式。

Windows 上 `cargo +1.98.1 fmt --all`、`cargo +1.98.1 ncheck -p monocode-gpui --all-targets`、`cargo +1.98.1 ntest -p monocode-gpui --bin monocode-gpui -- notifications:: --test-threads=1` 和 `cargo +1.98.1 nbuild -p monocode-gpui --bin monocode-gpui` 均以状态码 0 结束。通知测试通过 14 项，PCM 独立测试通过 1 项。Linux clippy 由 PR CI 验证。
