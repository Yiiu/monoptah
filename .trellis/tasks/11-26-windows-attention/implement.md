# Implement: Windows attention

Branch: lane `feat/agent-overview-tree-windows-attention`, merged back into `feat/agent-overview-tree`.
Specs: `11-25-agent-status-core/design.md` §7 and `research/windows-attention.md`. They give the
GPUI APIs, crates, HWND access and risks; do not re-research them.

## Where the existing code is

- **Platform bridge** (`notifications/platform.rs:17-53`): free functions. macOS is wired through `#[path = "platform_macos.rs"]`. Add `platform_windows.rs` with the same signatures.
- **GPUI toast API** (`vendor/gpui-pre/src/app.rs:1840-1876`):
  - `set_app_identity`
  - `show_system_notification`
  - `on_system_notification_response`
- **Delivery policy** (`notifications/delivery.rs`):
  - `should_notify` at :137
  - `Target::from_tag` at :170, currently macOS-only
  - `dock_badge_count`
- **Host** (`shell/notifications.rs`):
  - `start_native_notifications` at :322
  - the `drain_clicks` poll at :330
  - `open_notice_target` at :719
- **Sound** (`sound.rs`): `afplay` only today.

## Steps (one commit each)

1. **Dependencies and cfg.**
   - Add the `windows` 0.62 features listed in the research, plus `raw-window-handle` for Windows, in `crates/monocode-gpui/Cargo.toml`.
   - Update `Cargo.lock`.
   - Widen `cfg_attr(not(macos), allow(dead_code))` to `not(any(macos, windows))` in `platform.rs:9` and `delivery.rs:85-87`.
2. **Portable logic, with tests.**
   - Enable `Target::from_tag` on Windows.
   - Add a 5 s per-target dedupe in `delivery.rs`.
   - Format the toast title and body as "{project} · {session}".
   - Build the badge pixel buffer: a 16×16 RGBA amber circle with 3×5 digits and "9+".
   - Test filter: `notifications::`.
3. **Identity.** In `main.rs`, call `cx.set_app_identity(...)` on Windows. Use `com.monoptah.native` when `native-install.json` sits next to the exe, otherwise `.dev`.
4. **`platform_windows.rs`.**
   - `show`: the host posts through `cx.show_system_notification` with an `Open` action.
   - Clicks: `on_system_notification_response` → `Target::from_tag` → the existing click queue. Mark the target seen.
   - `permission` → Granted.
   - `open_settings` → `ms-settings:notifications`.
   - `set_dock_badge`: `ITaskbarList3::SetOverlayIcon` on the HWND captured from the shell window (via `raw-window-handle`), with an HICON from the step 2 buffer.
   - `FlashWindowEx(FLASHW_TRAY | FLASHW_TIMERNOFG)` when a NeedsYou notice is dispatched while the window is inactive.
5. **Sound.** Enable the cue WAV writer on Windows and play it with `PlaySoundW(SND_FILENAME | SND_ASYNC | SND_NODEFAULT)`.
6. **Manual verification** on Windows 11, covering the acceptance list in `prd.md`.

## Checks

Run on Windows:

```bash
cargo fmt --all
cargo +1.98.1 ncheck -p monocode-gpui --all-targets
cargo +1.98.1 ntest -p monocode-gpui --bin monocode-gpui -- notifications:: --test-threads=1
```

Linux clippy runs in CI. Make sure no new Windows-only dead code is left behind.

## Hand-over

- A checklist mapping requirements 1–9 to file:line and their test.
- The results of the manual pass.
- Merge back to `feat/agent-overview-tree` and push.
