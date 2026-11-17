# Native Persistence Safety Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development（用户已选择子代理分工）。Steps use checkbox (`- [ ]`) syntax for tracking. 本仓按批次完成编辑后统一验证；每个请求一次整体审查，不按每个小步骤重复重链 GPUI 测试二进制。

**Goal:** 用可复现的前后对比衡量保存成本，消除已确认的 Inbox 等锁入口，并为下一批普通会话增量存储提供实测依据。

**Architecture:** 第一批保留 transcript 格式、650 ms 节奏和事务边界，先给旧保存路径加轻量测量并冻结基线。Inbox 使用一个绑定固定 key 的后台设置保存组件，每个实例只有一个执行值和一个待保存最新值。普通会话分块存储及迁移另做计划，不在此批顺手改变数据格式。

**Tech Stack:** Rust、GPUI、现有 rusqlite/serde_json、Node 内置测试、macOS 条件编译的 `proc_pid_rusage`；所有 Native 编译使用 `native-dev` profile。

**Spec:** `docs/superpowers/specs/2026-11-16-native-transcript-write-amplification-design.md`

## Global Constraints

- Native only；不修改 `src/` 或 `src-tauri/` 的功能。
- 开发和验证使用 `native-dev` profile；新 worktree 首次 cargo 命令前克隆现有 target 缓存。
- 一批编辑完成后统一验证一次；修复后只重跑失败项。普通任务不反复跑完整 GPUI 测试。
- 保留 `monocode.*`、`monocode-host`、包名、crate 名、存储中的 `INTERRUPT_MESSAGE` 等既有标识。
- 不读取或输出用户消息正文、token、环境凭据作为诊断数据；fixture 全部合成。
- 不修改生产数据库做性能实验；只使用临时目录的合成库及 Native Dev 独立数据。
- 不通过关闭 journal、降低 synchronous、禁止 fsync、无限延迟 checkpoint 或无限积压内存来换取数字下降。
- 初始方案保留 650 ms 的普通保存节奏和既有强制保存边界，不用延长丢失窗口作为优化手段。
- 流式片段尚未提交时允许仍处于内存中；不能承诺异常断电后保留所有尚未提交的 token。
- 未成功提交的消息不能报告为已持久化、已接收或已完成。
- 队列出队、消息写入和接收凭证必须保留在同一个 SQLite 数据库的同一个事务内。
- 每个请求只做一次整体代码审查；迁移、数据删除和降级转换另做专门的数据安全审查。
- 本规划不授权 commit、push、合并、发布或对生产数据库执行转换。
- 每批都必须提供同一合成数据、同一事件序列、同一构建及 SQLite 配置下的修复前后对比；缺失或未完成的测量明确标记，不填 0、不推算为实测结果。
- 代码遵循周边 Rust 命名和错误处理风格；用小函数、明确的数据结构和最少抽象实现，不引入通用任务框架、插件式注册或为未来用途预建接口。
- 测试使用可移植的同步屏障和 `secondary-...` 快捷键；macOS 专属计数隔离在条件编译内，普通正确性测试可在 Linux 运行。

## Review Focus

- 数据库锁被占用：UI 仍处理事件，后台才能等待 Store；Task 3 的屏障测试覆盖。
- A 已保存、B 正在保存、再次请求 A：最终必须恢复 A，不能因缓存去重误丢；Task 3 的 A/B/A 测试覆盖。
- 关闭窗口、退出、重启时仍有保存：最新请求有独立生命周期，失败不能报成功；Task 3 的 drain 测试覆盖。
- BUSY、只读或提交失败后重试相同值：脏值保留，旧失败不能覆盖新成功；Task 1、3 的失败和 generation 测试覆盖。
- 前后数据/配置不同、计数缺失、预算用尽：比较器拒绝给出改善结论，不能把缺失计数当零；Task 2 的比较器测试覆盖。

---

## 文件边界与维护要求

新增模块只覆盖现有需求：

- `crates/monocode-core/src/store/save_stats.rs`：数字统计；不保存正文、不增加第二次序列化。
- `crates/monocode-core/examples/transcript_save_probe.rs`：确定性的合成基准；拒绝用户数据库输入。
- `crates/monocode-gpui/src/persistence_log.rs`：60 秒数值汇总，复用已有有界日志。
- `crates/monocode-gpui/src/ui_store_saves.rs`：一个固定 key 的后台设置保存，明确的状态和回执。
- `scripts/compare-native-persistence.mjs` 与 `.test.mjs`：读取两份结果、验证可比性、生成对比；仅使用 Node 内置模块。
- `docs/native/persistence-profiling.md`：运行方法、原始结果位置、实际前后对比和未完成项。

修改现有 Store/运行时只为采集已有操作的统计；修改 Inbox/退出入口只为接入保存组件。沿用 `Store::save_setting`、settings 已有去重/失败语义和 JSON 格式。禁止扩展为多 key 注册表、任意数据库任务系统、trait 层级或新一套 settings 文件。错误显式返回，注释解释顺序、并发和兼容原因，不复述代码；不为风格重排无关大文件。

## 前后对比执行协议

1. 在相同测量工具下先运行 Before：只加入统计和探针，尚未修改保存行为。记录源码 SHA、未提交 diff SHA、工具链、Cargo.lock SHA、profile 和测量工具版本。保存原始 JSON 后才改业务行为，不需要为建立基线擅自 commit/push。
2. fixture 固定 ID/时间戳/UTF-8 字节和 SHA；workload 固定事件、内容和逻辑时间。Before/After 各用全新临时库，使用同一设备、profile 和 SQLite 配置。普通场景为四次各 1024 字节追加，逻辑时间 0/650/1300/1950 ms，2600 ms 结束。不能因旧版慢就减少事件。
3. 分别记录 setup/转换、热保存、最终保存、结束后一次相同策略的 checkpoint；关闭数据库后取总写入。报告 WAL 峰值和实际总耗时，不把写入挪到测量窗口外。
4. 小样本报告每次值、总量、最大值；不把四次旧保存和百次新保存的 p95 比较。p95 < 50 ms 是第二批至少 100 次补充小样本的待测目标。
5. 每次探针执行的累计进程写入预算为 512 MiB，停止后记录实际值和未完成场景；同时汇总所有 Before/After 执行的总成本。旧大库场景仅运行一次，需要重跑时必须说明先前结果为何不可用。
6. 重新打开数据库验证 rows、原始未知元数据及队列/接收凭证的指纹。输出只含指纹和数值，不能包含消息正文、路径或凭据。
7. 第一批结果预期是 Inbox UI 响应改善；transcript 仍完整写 blob，不宣称达到第二批 95% 写入降低目标。测量工具或配置变化时重新建立配对基线，不能拼接不相容结果。

### Task 1: 保存统计与合成探针

**Files:**
- Create: `crates/monocode-core/src/store/save_stats.rs`
- Create: `crates/monocode-core/examples/transcript_save_probe.rs`
- Create: `crates/monocode-gpui/src/persistence_log.rs`
- Modify: `crates/monocode-core/src/store.rs:1013`
- Modify: `crates/monocode-core/src/store/shared.rs:1252`
- Modify: `crates/monocode-gpui/src/runtime/persistence.rs:45`
- Modify: `crates/monocode-gpui/src/runtime.rs`（保存原因参数的调用点）
- Modify: `crates/monocode-gpui/src/native_log.rs:147`、`src/main.rs:463`
- Modify: `crates/monocode-core/Cargo.toml`（探针的 macOS 条件 dev dependency 复用仓内已有 libc 版本）
- Test: `save_stats.rs`、probe example、`store/shared/tests.rs`、GPUI 新模块内测试。

**Interfaces:**
- Produces: `SaveReason::{Routine, Boundary, Acceptance, Final}`。
- Produces: `SaveSample`，字段为 `reason`、`history_bytes_read: u64`、`serialized_bytes: u64`、`changed_rows: usize`、`commit_count: u64`、`lock_wait: Duration`、`lock_held: Duration`、`projection: Duration`、`encoding: Duration`、`commit: Duration`、`succeeded: bool`。
- Produces: `Store::take_save_sample(&mut self) -> Option<SaveSample>`；必须在当前保存的锁作用域内取出。GPUI 补入锁计时，core 探针缺失的 UI 锁计时在结果中为 `null`。
- Produces: `persistence_log::record(sample: SaveSample)` 和 `persistence_log::start(cx: &mut gpui::App)`；`MONOCODE_PERSISTENCE_STATS=1` 启用 60 秒汇总。
- Produces: probe CLI `--scenario <id>`，仅支持 `append-1m`、`append-10m`、`append-40m`、`same-value`、`early-tool`、`large-tool`、`long-answer`、`parallel`；每次创建临时库，stdout 输出一个版本为 1 的结果 JSON，不提供用户库路径参数。

- [ ] **Step 1: 写计数与失败测试。** `utf8_lengths_are_bytes` 断言 `"中文🙂".len() == 10` 并与计数相同；`failed_commit_keeps_pending_rows` 检查提交失败没有成功 commit、pending 未清空、重试可恢复。使用小合成库，不写真实消息。
- [ ] **Step 2: 在已有读、投影/合并、编码、提交位置累加统计。** 用现有缓冲区长度与局部计时，不再读/clone/serialize 历史；锁计时放 `PendingSave`。保留原有 Store 写入接口，在 `flush` 参数中加入 `SaveReason` 并更新所有调用点。
- [ ] **Step 3: 实现固定场景生成与完整性检查。** fixture 的 ID、时间戳、未知字段固定，按实际 UTF-8 字节报告历史大小；除 1/10/40 MiB 已完成历史，还覆盖 5 MiB 活动工具、长回答、早期工具和并发会话。保存前后及重启后的内容做指纹验证，不打印正文。
- [ ] **Step 4: 实现 I/O 预算和汇总。** macOS 复用 `memory_log.rs:57` 的 `proc_pid_rusage` 模式，读 `ri_diskio_byteswritten`。达到 512 MiB 后不提交下一个保存；单次操作可能超出剩余预算，必须如实记录。其他系统无 OS 计数则为 `null`，默认跳过 40 MiB 大场景。日志只允许固定 persistence 数字 target，默认关闭；不取代 auto-checkpoint hook，不逐次写日志。
- [ ] **Step 5: 添加探针保护测试。** `budget_stops_before_next_save` 使用假计数器；`missing_io_is_null` 检查缺失；`stats_do_not_serialize_payloads` 检查输出白名单。新增 GPUI 测试放 `mod persistence_safety_tests`，core 测试归 `store::`，probe 内测通过 `--example transcript_save_probe` 运行。测试命令集中在 Task 4，不在每次编辑后重链。

### Task 2: 自动比较结果并冻结 Before

**Files:**
- Create: `scripts/compare-native-persistence.mjs`
- Create: `scripts/compare-native-persistence.test.mjs`
- Modify: Task 1 的 probe JSON 输出。
- Create: `docs/native/persistence-profiling.md`

**Interfaces:**
- Consumes: 两份 `schema_version: 1` 的 probe 结果，含源码/构建标识、fixture/workload 指纹、SQLite 配置、场景状态、样本数、分阶段数值及完整性指纹。
- Produces: `export function compareRuns(before, after)` 返回 `{ comparable: boolean, reasons: string[], cases: object[] }`；每个 case 列 before/after 原值及改善率。源码 SHA 可不同，fixture/workload/toolchain/profile/SQLite 配置必须相同。
- Produces: CLI `node scripts/compare-native-persistence.mjs --before <json> --after <json> --out <json>`；不可比较或数据不一致退出 1，完整可比较退出 0。性能是否达标独立列出，第一批不强制 95% 目标。

- [ ] **Step 1: 写比较器测试。** 合成一对相同 manifest、写入分别为 1000/100 字节的结果，断言改善率为 0.9；分别改变 fixture SHA、workload SHA、profile 和 SQLite 配置，断言 `comparable === false`。`missing_counts_are_not_zero` 断言改善率为 `null`；`zero_before_has_no_percentage` 同样为 `null`；`unfinished_or_corrupt_run_is_rejected` 检查预算用尽或完整性不匹配不能产生达标结论。
- [ ] **Step 2: 实现比较器与结果字段验证。** 只使用 Node 的 fs、path 和内置测试；不引入图表/benchmark 框架。样本数量不相同时不计算配对耗时改善率；保留原值。JSON 中输出总写入、编码量、提交数、保存耗时、锁计时和完整性检查，缺失字段明确列出原因。
- [ ] **Step 3: 在 Task 3 业务改动前冻结 Before。** 先完成测量工具的格式/lint/对应测试，再仅对旧保存路径运行受预算约束的 probe。记录 source SHA + diff SHA，保存原始结果和 manifest，文档注明历史 549.76 GB 诊断不属于该基线。旧实现大场景不重复采样。若测量工具失败，只修测量工具并重新建立受影响基线，不能跳过此步骤。
- [ ] **Step 4: 写入维护文档。** 固定原始结果和比较结果的目录约定；列“场景、Before、After、绝对差值、改善率、样本数、完整性、完成状态”。尚未运行的字段写“待测”，不能填估计数字；记录当前批次仍未改变 transcript blob 的事实。

### Task 3: 最小后台保存组件与 Inbox 接入

**Files:**
- Create: `crates/monocode-gpui/src/ui_store_saves.rs`
- Modify: `crates/monocode-gpui/src/inbox.rs:355`、`:408`、`:521`、`:555`、`:588`、`:641`
- Modify: `crates/monocode-gpui/src/main.rs:146`、`:164`（模块/global 初始化及退出协调）
- Test: 新组件、Inbox 和退出协调的 `mod persistence_safety_tests`。

**Interfaces:**
- Consumes: `runtime::Database`，现有 `Store::save_setting(&self, key: &str, value: &str) -> Result<(), String>`。
- Produces: `UiStoreSaves::new(store: Database, key: &'static str, executor: gpui::BackgroundExecutor) -> Self`；一个实例只绑定一个 key，Inbox 的实例由 app global 持有，key 为 `native-inbox-state`。
- Produces: `UiStoreSaves::save_latest(&self, value: String) -> Result<SaveTicket, String>`；不在 UI 取得 Store 锁。
- Produces: `SaveTicket { generation: u64, result: async_channel::Receiver<Result<SaveOutcome, String>> }`，`SaveOutcome::{Committed { generation: u64 }, Superseded { by: u64 }}`。
- Produces: `UiStoreSaves::drain(&self) -> async_channel::Receiver<Result<(), String>>`；等待调用时已接收的最新设置，不阻塞 UI。失败值通过再次 `save_latest` 重试，不另建 retry API。
- Produces: `Inbox::save(&mut self, cx: &mut Context<Self>)`；序列化 ViewState、异步保存、按当前 generation 显示错误。原 JSON 格式及设置 key 不变。

- [ ] **Step 1: 写状态/顺序测试。** `a_b_c_coalesces_to_a_c` 用屏障让 A 先开始，B/C 等待，最终只提交 A/C，B 收到 Superseded；`a_b_a_commits_a_last` 先成功 A、再阻塞正在执行的 B、再次请求 A，最终内容必须为 A；`failed_identical_value_can_retry` 验证失败值不能被去重；`idle_successful_duplicate_skips_store` 验证只有无其他不同值在途时才能跳过锁。
- [ ] **Step 2: 实现简单状态机。** 字段只保存 committed、executing、pending latest、generation 和回执；每实例至多一个执行值和一个待执行值。worker 在后台取得 Store 锁；等待 Store 时不持内部状态锁。完成后释放 Store，再处理最新值；沿用已有 settings 去重，不加注册表或通用 job trait。
- [ ] **Step 3: 写主线程/生命周期测试。** `held_store_keeps_ui_running` 在后台持 Store 锁，释放前必须收到 UI sentinel；`owner_close_keeps_latest_save` 丢弃视图后仍提交最新值；`old_failure_does_not_override_new_success` 验证 generation；`drain_waits_for_latest_and_reports_failure` 检查成功和故障。用通道屏障，不使用固定长 sleep，不在 UI 等后台结果。
- [ ] **Step 4: 替换 Inbox 六个保存调用点。** `serde_json::to_string(&self.state)` 后提交共享组件，在 weak entity 回调更新错误；只有最新 generation 可以改变保存错误。将保存错误与其他 Inbox 错误区分，成功不能清掉无关网络错误。刷新后台获取 sessions 的现有方式保持。
- [ ] **Step 5: 写退出回归并接入 drain。** 测试 `quit_waits_for_latest_setting` 和 `failed_drain_keeps_app_open`。退出/重启先停止接收新的设置变更，等待该保存器 drain 成功后再执行既有 provider shutdown 和 quit/restart；失败时恢复可重试状态并保持进程及 providers，避免先杀进程后又因保存失败留在半关闭状态。退出期间 UI 仍前进，不调用 UI join。实现时复用现有错误展示，不创建新的复杂对话流程。
- [ ] **Step 6: 记录其他同步调用。** 在 profiling 文档列出 `notes.rs`、`conversation.rs:2515`、`conversation/mono.rs:341` / `:465` 和 shell 中查到的 UI Store 调用。第一批只接入 Inbox；需要事务性业务回执的操作不塞进设置保存器。向 Task 4 提供覆盖清单。

### Task 4: After、项目检查与一次整体审查

**Files:**
- Modify: `docs/native/persistence-profiling.md`
- Test: Tasks 1–3 的新增模块、probe example、原有 runtime persistence/shared-store 测试。

**Interfaces:**
- Consumes: 冻结的 Before、同 manifest 的 After、自动比较 JSON、真实 UI 证据。
- Produces: 需求 → `file:line` → test → 前后结果 的交接清单，明确通过、失败和未完成项。

- [ ] **Step 1: 本批业务编辑完成后统一验证。** Task 1–2 建立 Before 的测量阶段和 Task 3 修正业务的阶段各完成所有编辑后集中跑一次，不能每个小步骤反复测试。macOS 新 worktree 在首个 cargo 命令前用 `cp -c -R` 克隆已存在的 native target；保留源缓存，不清理其他任务文件。

```bash
cargo fmt --manifest-path crates/monocode-gpui/Cargo.toml --all
cargo clippy --locked --manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev -p monocode-core -p monocode-gpui --all-targets -- -D warnings
cargo test --locked --manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev -p monocode-core store::
cargo test --locked --manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev -p monocode-core --example transcript_save_probe
cargo test --locked --manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev -p monocode-gpui persistence_safety_tests
cargo test --locked --manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev -p monocode-gpui runtime::persistence::tests
node --test scripts/compare-native-persistence.test.mjs
```

预期直接退出 0，匹配到实际测试，不能将零测试当通过证据。只重跑失败或受修正影响的项。合并 main/发布前按仓规完整 `npm run native:check` 每批一次；Linux/Windows 的检查由已有 CI 完成，不能仅凭本机通过声称跨平台已通过。

- [ ] **Step 2: 跑同负载 After。** 使用与 Before 相同的测量工具、manifest、配置和设备，命令形式为 `cargo run --locked --manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev -p monocode-core --example transcript_save_probe -- --scenario append-1m`；其他场景仅替换已定义 id。保存 JSON、总写入预算及完整性结果，用比较器生成对比。第一批写入量仍受 blob 影响，如实标注，不抬高或更换基线。
- [ ] **Step 3: 验证真实 UI。** 按仓规使用干净且匹配 origin 的 worktree 运行 `npm run native:dev -- --detach`，使用独立 Dev 数据和合成场景。Before/After 分别在相同的 DB 锁占用条件下刷新 Inbox、输入、滚动、选取复制，再关闭重开/正常退出重启；记录 UI sentinel 和实际体验。只有在操作已获授权时才推送准备该 worktree；未满足时标记真实 UI 待测，不将生产 app 当压力测试目标。
- [ ] **Step 4: 一次整体代码审查。** 提供原始要求、spec、diff、需求到代码/测试/结果的清单。核对实现是否使用最少状态、复用已有代码、命名易懂、无多余抽象；核对 UI 等锁、A/B/A、错误、关闭及统计隐私。reviewer 只为具体疑点运行测试；证实问题修正后只重跑相关项。
- [ ] **Step 5: 交付实际对比。** 记录绝对值与比例、完整性、样本数、旧版大场景运行次数、全部实验写入成本和局限。代码修改后必须展示 Before/After，不能仅给“感觉流畅”。没有实测的 95% 或 p95 仍写待验收；提交、推送、合并和发布仅按已有/后续明确授权执行。

## 第二批：增量存储单独计划的固定输入

第一批的基准是下一批设计输入。下列工作属于另一份数据格式实施计划，第一批不执行迁移：

1. **存储与块内分片。** 同一个主 SQLite 库新增普通 Native 存储模式，复用 rich-block 适配和 changed-block 比较；不把普通会话标为 Mono。已完成历史和 5 MiB 活动 payload 两种基准共同决定布局；只按消息拆表不算完成。
2. **稳定绝对行号与云端继续。** 保持 `moved_at_row`、返回本地 gate、turn receipts、编辑/截断、follow-up 语义；UI window 不改变原点，迁移前已有行号有映射规则。
3. **reader 完整性。** 搜索 revision 失效、Today 回填、side-thread 清理和快照、移动/恢复/已定位的复制导出入口均覆盖；全历史访问与分页分开，不声称验证了尚未定位的通用导出 API。
4. **稀疏更新与冲突。** 早期工具加尾部 token 不重写中间内容；事务内检查 rich revision，隐藏/未知字段变化也构成冲突。缺失或重复 block ID 有明确处理。
5. **迁移与旧版本。** 固定格式版本、旧 writer 写入屏障、一致性备份、原子切换、FULL/BUSY/中断恢复及保留最新内容的降级导出。新表本身不能阻止旧客户端继续写 blob；不持续双写完整 blob。
6. **上线和评审。** 合成库 → Native Dev → 新本地会话 → 按需转换旧本地会话；不启动时批量迁移，既有 cloud/remote/TUI/Mono 各自规则保留。迁移/删除/降级路径单独审查。
7. **复用同一前后协议。** 完整 40 MiB 小追加场景累计进程写入降低至少 95%；10→40 MiB 已完成历史不增加逻辑热写入；大活动消息不反复整条重写；热保存 p95 < 50 ms 单独补充测量。最终 checkpoint、转换成本和内容一致性必须展示，未通过时不能标记根治完成。
