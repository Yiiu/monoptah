# 实现与验证步骤

1. 以最新主线为基线，在独立工作树实施并保留当前工作区已有的 Reclaude 启动补丁。阅读本任务的 PRD、设计和两个 Trellis context 清单。
2. 在现有 `provider/follow_up.rs`、`conversation/follow_up.rs` 的测试框架里构造忙碌 Codex、本地暂停队列及 Steer。补充对四段事件的脱敏诊断。按复现结果判断写入前还是等待确认期间受阻。若原现场无法复现，记录已验证的路径，分别用可控输入验证它们。
3. 补齐 Steer 的状态反馈及收到晚到确认时的一次性交付。覆盖明确拒绝、没有确认、旧回复晚到、回合结束、跨会话并行和加载已有的暂停队列。用原有收据和 Codex 的消息 ID 核对交付。测试两条同文消息、旧版本没有客户端 ID 回显，以及停止或重连后收到前一轮的迟到确认。
4. 用不回中断确认和只回确认但未到终态的 provider 分别复现 Stop 卡住，增加有界收尾和对旧连接的隔离。验证正常确认路径、旧连接退出或终态迟到、停止后同一会话发送下一条、附件和未确认队列保留、另一会话继续运行。
5. 使用 Native `native-dev` 配置检查受影响的范围，在 `crates/monocode-gpui` 目录执行以下命令。

   ```powershell
   cargo fmt --all -- --check
   cargo ntest -p monocode-core --lib provider::follow_up
   cargo ntest -p monocode-core --lib provider::parked
   cargo ntest -p monocode-core --lib provider::tasks
   cargo ntest -p monocode-gpui conversation::follow_up
   cargo nclippy -p monocode-core -p monocode-gpui --all-targets -- -D warnings
   ```

   将 Stop 回归测试放在对应模块，并在 Windows 运行其过滤器。现有的 Unix 专用进程测试在 Unix 或 CI 上执行。记录基线已有的 Clippy 问题，检查新增代码与当前链路。
6. 用隔离数据的 Windows Native 构建重放本次现场，记录从点击、派发、确认到下一轮恢复的实际耗时，并确认可反复操作。若用户体验仍是界面无响应且日志显示正常确认，继续定位 UI 更新和输入状态，别把队列恢复的结果当作整个会话已修复的证据。
