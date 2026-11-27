# Research: Windows attention (toast, taskbar badge, flash, sound)

- **Query**: How to add, on Windows 11 with the window in the background: (a) a toast with an "Open" button that opens the session and stays in Action Center, (b) a numeric taskbar overlay badge, (c) taskbar flashing until focus, (d) a sound. Cover the current macOS interface, crates, AUMID, HWND, flashing, sound and activation.
- **Scope**: mixed (repo + vendored gpui + Microsoft Learn)
- **Date**: 2026-11-25

## Key finding first

The vendored GPUI already ships a **Windows toast backend with click routing**, and a flash API:

- `App::set_app_identity(id, name)`, `App::show_system_notification(SystemNotification)`, `App::dismiss_system_notification(tag)` and `App::on_system_notification_response(cb)`: `vendor/gpui-pre/src/app.rs:1840-1876`. Types are at `vendor/gpui-pre/src/platform.rs:646-680`: `SystemNotification { tag, title, body, actions: Vec<SystemNotificationAction{id,label}> }` and `SystemNotificationResponse { tag, action_id: Option<_> }` (`None` means the toast body was clicked).
- The Windows implementation is in `vendor/gpui-pre-windows/src/system_notifications.rs`:
  - Builds `ToastGeneric` XML with two `<text>` elements (title, body) and `<action content=label arguments=id>` (lines 157-187).
  - Always sets `<audio silent="true"/>` (184-186), so the app must play its own sound.
  - Hashes the GPUI tag into a 16-hex Windows tag (56-63). The `Activated` handler sends `{original tag, action_id}` through a channel (65-83), and the response is delivered on the main thread (105-127).
  - Keeps every shown `ToastNotification` in `active_toasts` (89).
  - Without package identity it calls `register_app_user_model_id`, which writes `HKCU\Software\Classes\AppUserModelId\{id}` with `DisplayName` (191-198), then `CreateToastNotifierWithId(id)` (138-150). If `set_app_identity` was never called, it logs a warning and shows nothing (141-147).
- `set_app_identity` on Windows also calls `SetCurrentProcessExplicitAppUserModelID` (`vendor/gpui-pre-windows/src/platform.rs:893-908`). It is a no-op when the process has package identity (`platform.rs:1453`, `GetCurrentPackageFullName`).
- `Window::request_attention()` exists (`vendor/gpui-pre/src/window.rs:6724-6727`). On Windows it calls `FlashWindowEx` with `FLASHW_ALL`, `uCount: 1` and returns early if the window is active (`vendor/gpui-pre-windows/src/window.rs:868-887`). That is **one flash, not "until focused"**.
- `Window::play_system_bell()` (`window.rs:7077`) calls `MessageBeep(MB_OK)` on Windows (`gpui-pre-windows/src/window.rs:1075-1078`).
- `Window` implements `HasWindowHandle` (`vendor/gpui-pre/src/window.rs:7667-7671`). On Windows this is a `Win32WindowHandle` holding the HWND (`vendor/gpui-pre-windows/src/window.rs:595-603`).
- Not in GPUI: taskbar overlay badge (no `ITaskbarList`/`SetOverlayIcon` anywhere in `vendor/gpui-pre-windows/src`), `register_url_scheme` (stubbed with "unimplemented", `platform.rs:1125-1126`), and a toast-permission query.
- monocode-gpui calls none of these APIs today (grep for `set_app_identity|show_system_notification|request_attention|on_open_urls` in `crates/monocode-gpui/src` finds nothing).

## Findings

### 1. The current interface a Windows backend must implement

| File | Role |
|---|---|
| `crates/monocode-gpui/src/notifications/platform.rs` | The OS bridge as **free functions**, with no trait. `#[cfg(target_os="macos")] #[path="platform_macos.rs"] mod implementation; pub use implementation::*;` (17-22). Non-macOS stubs are at 24-53. |
| `crates/monocode-gpui/src/notifications/platform_macos.rs` | The macOS implementation (UNUserNotificationCenter). |
| `crates/monocode-gpui/src/notifications/delivery.rs` | Policy: `Permission`, `Target`, `should_notify`, `clip_body`, `dock_badge_count`. |
| `crates/monocode-gpui/src/shell/notifications.rs` | The host: startup, poll loop, dispatch, opening the session. |
| `crates/monocode-gpui/src/sound.rs` | Cue synthesis; plays only through `afplay` on macOS. |

These are the functions a Windows module has to provide, with the same signatures:

- `pub struct Notification { tag, subtitle, body, sound: bool }` (`platform.rs:8-15`). Its fields are marked `cfg_attr(not(macos), allow(dead_code))`, and Windows must be excluded from that.
- `initialize() -> bool` (main thread). It is called from `start_native_notifications` (`shell/notifications.rs:322-327`). Returning `true` sets permission to `Prompt` and starts a `permission()` probe.
- `permission() -> Receiver<Permission>` and `request_permission() -> Receiver<Permission>`. Note that `delivery::should_notify` (`delivery.rs:137-146`) only allows `Prompt | Granted`. Today Windows reports `Unsupported`, so every notice falls back to sound-only (`shell/notifications.rs:662-676`), and on Windows that sound is silent too.
- `show(Notification) -> Receiver<Result<(),String>>`. The host waits up to 5 s and plays the sound cue as a fallback when the result is not `Ok` (`shell/notifications.rs:683-716`).
- `drain_clicks() -> Vec<Target>` is polled **once per second** (`shell/notifications.rs:330-347`), and each target goes to `open_notice_target` (719-788). That function closes Settings and app views, opens the session (or the Mono session) and calls `window.activate_window()` (735, 783).
- `set_dock_badge(count: usize)` is called from `start_native_notifications` (323), `process_notification_inputs` (421-424) and `sync_notices` (534-537). The count comes from `delivery::dock_badge_count` (`delivery.rs:215-223`): the number of sessions waiting for input. It has **no `Window` parameter**, which matters for the HWND (section 3).
- `dismiss(tag: &str)` is used for reminders (`shell/notifications.rs:803, 866`).
- `open_settings() -> Result<(),String>` is used by Settings (`settings/notification_preferences.rs:364`). The `Permission::Unsupported` UI branch is at `:370`.
- Tags: `Target::tag` returns `monocode-session:{id}:{seq}` or `monocode-reminder:{id}:{due}` (`delivery.rs:159-168`). `Target::from_tag` is `#[cfg(target_os="macos")]` (`delivery.rs:170-182`) and would have to be enabled for Windows. `Permission::Granted/Denied` carry the same `cfg_attr` dead-code allowance (`delivery.rs:84-88`).
- Two more `show` callers: `shell/web_link.rs:183,197`.
- On macOS, clicks are collected by a delegate into `static CLICKS: Mutex<VecDeque<Target>>` (`platform_macos.rs:25, 44-61`). The "Show" action and the default click are both accepted. The title is fixed as "Monoptah" and the subtitle is the session title (`platform_macos.rs:170-176`, `shell/notifications.rs:683-692`). The requested Windows copy, "project · session waiting for you", would need the host to build a different title. `project_name(&summary.cwd)` is already used for reminders (`shell/notifications.rs:591`).

### 2. Crates and AUMID

Lockfile: `crates/monocode-gpui/Cargo.lock`, which is the native workspace.

| Crate | In lock? | Who pulls it | Click → argument back to the app? |
|---|---|---|---|
| `windows` 0.62.2 | yes | `gpui-pre`, `gpui-pre-windows` (Windows target), wgpu-hal, accesskit | Direct WinRT `ToastNotification.Activated`, which GPUI already uses |
| `windows` 0.61.3 | yes | only `tauri-winrt-notification` | — |
| `windows-sys` 0.61.2 | yes | `monocode-core` (`crates/monocode-core/Cargo.toml`, `[target.'cfg(windows)']`: Foundation, Security, JobObjects, Threading) | No COM interfaces, so it cannot do `ITaskbarList3` |
| `notify-rust` 4.18.0 | yes | **only `gpui-pre-linux`**, so it is not compiled for Windows | Windows backend = tauri-winrt-notification |
| `tauri-winrt-notification` 0.7.3 | yes (through notify-rust) | Linux path only | `Toast::on_activated(FnMut(Option<String>))`, the same in-process `Activated` event. It would compile a second `windows` (0.61) on Windows |
| `winrt-toast`, `win-toast`, `rodio`, `cpal`, `single-instance`, `interprocess` | **no** | — | — |
| `raw-window-handle` 0.6.2 | yes | monocode-gpui declares it **only under `cfg(target_os="macos")`** (`crates/monocode-gpui/Cargo.toml:73`) | — |

- monocode-gpui has **no `windows`/`windows-sys` dependency** today (`crates/monocode-gpui/Cargo.toml:8-83`).
- `gpui-pre-windows` enables these `windows` 0.62 features (`vendor/gpui-pre-windows/Cargo.toml`): `Win32_UI_Shell`, `Win32_UI_WindowsAndMessaging`, `Win32_Graphics_Gdi`, `Win32_System_Com`, `Win32_Media`, `UI_Notifications`, `Data_Xml_Dom` and more. A direct `windows = "0.62"` dependency in monocode-gpui unifies with it; only `Win32_Media_Audio` (for `PlaySoundW`) would be new.
- API locations in `windows-0.62.2`, all confirmed:
  - `ITaskbarList3::SetOverlayIcon` is gated on `Win32_UI_WindowsAndMessaging` (`src/Windows/Win32/UI/Shell/mod.rs:47199-47200`), and `TaskbarList` CLSID is at `:56399`.
  - `FlashWindowEx` and `CreateIconIndirect` are in `Win32/UI/WindowsAndMessaging/mod.rs:702` and `:373`.
  - `PlaySoundW` is in `Win32/Media/Audio/mod.rs:111`, behind the `Win32_Media_Audio` feature.
  - `MessageBeep` is in `Win32/System/Diagnostics/Debug/mod.rs:546`.
- **Installer**: `scripts/native-installer.nsi:58-59` runs plain `CreateShortcut "$SMPROGRAMS\Monoptah Native.lnk"` and the same for `$DESKTOP`, with **no AppUserModelID** (stock NSIS cannot set `System.AppUserModel.ID` without a plugin or System-plugin COM calls). `scripts/package-native.mjs:322-358` stages `monocode-gpui.exe` plus `native-install.json` (`{app_id:"com.monoptah.native",kind:"nsis"}`) and runs `makensis`. `package-native-monoptah.mjs` reuses upstream's packaging for Windows (lines 7-8, 123-130).
- **AUMID without a shortcut**: GPUI uses the registry form (`HKCU\Software\Classes\AppUserModelId\<id>` + `DisplayName`, `system_notifications.rs:191-198`). The same method appears in tauri-winrt-notification's `examples/unpackaged_app.rs`, which also sets `IconUri` and `IconBackgroundColor`. So toasts work **without** a Start Menu shortcut, which also covers `npm run native:dev`. That script runs `target/native-dev/monocode-gpui.exe` directly (`scripts/dev-native.mjs:95-104`).
- Microsoft's older guidance says you "cannot raise a toast notification from a desktop app" without a shortcut that has an AUMID (<https://learn.microsoft.com/en-us/windows/win32/shell/enable-desktop-toast-with-appusermodelid>). The registry method is the newer alternative that Windows App SDK and Tauri use.
- The brand constant to reuse is `NATIVE_APP_ID = "com.monoptah.native"` (`crates/monocode-core/src/brand.rs:11`). Dev and installed builds can be told apart on Windows by `native-install.json` next to the exe (`crates/monocode-core/src/app_update/mod.rs:85-104`).

### 3. Taskbar overlay badge (HWND, HICON)

- HWND: `HasWindowHandle::window_handle(window)` → `RawWindowHandle::Win32(h)` → `h.hwnd`. The existing macOS pattern to mirror is `crates/monocode-gpui/src/traffic_lights.rs:87-95` (`RawWindowHandle::AppKit`). Other users are `file_editor/video.rs:90`, `monos/html.rs:366` and `shell/shortcuts/macos.rs:26`.
- `set_dock_badge` has no window. The Shell stores `window_active` from `window.is_window_active()` at `shell.rs:9634`, and `observe_window_activation` is at `shell.rs:9657-9660`. That is where the Shell holds a `Window` and could capture the HWND, for example into a `static`/`OnceLock`.
- `SetOverlayIcon(hwnd, hicon, alt)`: the icon should be 16x16 at 96 dpi, and passing `NULL` removes the overlay. The taskbar copies the icon, so the caller frees it with `DestroyIcon`. The call is **ignored when the taskbar uses small icons**, and overlays are lost when Explorer restarts (re-apply on `RegisterWindowMessage("TaskbarButtonCreated")`) (<https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-itaskbarlist3-setoverlayicon>). Windows 11 also has the user toggle Settings > Personalization > Taskbar > "Show badges on taskbar apps". GPUI does not handle `TaskbarButtonCreated` (no match in `vendor/gpui-pre-windows/src`).
- There is **no existing runtime badge-icon renderer**:
  - Nothing builds an HICON; GPUI only uses `LoadImageW` for the exe icon (`gpui-pre-windows/src/platform.rs:1661-1674`).
  - The closest pixel-generation code is the `image` crate (already a dependency, `Cargo.toml:23`), used in `visuals/dither.rs`, `project_background.rs` and others.
  - Building the icon takes: an RGBA buffer, then `CreateBitmap`/`CreateDIBSection` plus a mask, then `CreateIconIndirect`. Digits could be drawn from a tiny built-in 3x5/5x7 bitmap font, or the overlay could be a plain dot with the count in the alt text, plus a "9+" cap.
- Existing Windows `cfg` code in the crate:
  - `shell.rs:339` (`WINDOWS_CONTROLS`) and `shell.rs:355`
  - `shell/window_chrome.rs:9-24` (icons)
  - `shell/layout.rs:58`
  - `main.rs:229` (`LOCALAPPDATA` data dir)
  - `terminal/shell.rs:3-89`
  - `explorer.rs:503, 2233-2409`
  - `file_editor.rs:1708-1726`
  - `notifications/surface.rs:233-243` (`any(macos, windows)`)
  - `build.rs` embeds the icon only for the Windows target.

  None of these calls Win32 directly. The module pattern to follow is `platform.rs:17-22` (`#[path = "platform_macos.rs"]`).

### 4. Flashing

- GPUI's `window.request_attention()` flashes once (`FLASHW_ALL`, `uCount 1`, `window.rs:868-887`). To flash "until focused", call `FlashWindowEx` directly with `FLASHINFO { dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG, uCount: 0, dwTimeout: 0 }`. `FLASHW_TIMERNOFG` is 0xC, "flash continuously until the window comes to the foreground" (<https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-flashwinfo>). Call it only when `!window_active`.

### 5. Sound

- `sound.rs:9-35`: the only real playback is `afplay` under `cfg(all(macos, not(test)))`. The WAV writer `wave(cue)` is `cfg(any(macos, test))` (line 36), and files are cached in `data_dir()/sounds/*.wav`.
- Simplest Windows option with the same cues: enable `wave` for Windows and call `PlaySoundW(path, None, SND_FILENAME | SND_ASYNC | SND_NODEFAULT)`. This needs the `windows` feature `Win32_Media_Audio` and links winmm. Options without the custom cue are `window.play_system_bell()` (`MessageBeep(MB_OK)`, which needs a `Window`) or a toast with audio, which GPUI forces silent. `rodio`/`cpal` are not in the lock.

### 6. Activation and single instance

- The native app has **no single-instance or deep-link mechanism**: there is no `on_open_urls`, `register_url_scheme`, mutex, pipe or `monoptah://` handler in `crates/monocode-gpui/src` or `crates/monocode-core/src`. Only `monocode://pair` strings appear in Settings tests, and GPUI's Windows `register_url_scheme` is unimplemented. Only the Tauri app uses `tauri_plugin_single_instance` (`src-tauri/src/lib.rs:280`).
- With GPUI's toast path, a click while the app is running reaches the process **in-process** through `ToastNotification.Activated` → channel → `on_system_notification_response` callback on the main thread (`system_notifications.rs:65-83, 105-127`). No IPC is needed, and `response.tag` is the original `monocode-session:...` tag, so `Target::from_tag` can parse it.
- Without a COM activator (`CustomActivator` CLSID or `ToastActivatorCLSID`), a click after the process has exited does not relaunch the app with arguments. Microsoft's COM-activation model is described at <https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/notifications/app-notifications/app-notifications-quickstart>. A second launch from the shortcut would also start a second instance, because there is no single-instance guard.
- `window.activate_window()` on Windows uses the `SendInput(VK_MENU)` + `SetForegroundWindow` workaround (`gpui-pre-windows/src/window.rs:820-866`).

## Recommended minimal approach (for the implementer to weigh)

1. Startup (`main.rs`, in the `run` closure before `cx.open_window` at ~580): on Windows, call `cx.set_app_identity(<aumid>, "Monoptah Native")`. For the AUMID, use `com.monoptah.native` when `native-install.json` exists, otherwise something like `com.monoptah.native.dev`.
2. Add `notifications/platform_windows.rs` (wired like `platform_macos.rs`) that implements the same free functions:
   - `show`: needs `&App`, so either change `show` to take `cx` or post the toasts from the host. Calls `cx.show_system_notification` with title "project · session waiting for you", body = clipped question, and actions `[{id:"open",label:"Open"}]`.
   - `drain_clicks`: a `static` queue filled by an `on_system_notification_response` callback that `Target::from_tag` parses. Alternatively, call `open_notice_target` directly from that callback to avoid the 1 s poll delay.
   - `permission`: return `Granted` (or query `ToastNotifier.Setting()` through `windows` `UI_Notifications`).
   - `open_settings`: open `ms-settings:notifications`.
   - `set_dock_badge`: an `ITaskbarList3::SetOverlayIcon` on a stored HWND with a runtime 16x16 HICON.
3. Flash: call `FlashWindowEx(FLASHW_TRAY|FLASHW_TIMERNOFG)` when an input notice is dispatched with the window inactive. It could sit next to `set_dock_badge` or in `dispatch_notice`.
4. Sound: `PlaySoundW` on the cached cue WAV, extending `sound.rs`'s cfg to Windows.
5. Dependency: `[target.'cfg(windows)'.dependencies] windows = { version = "0.62", features = ["Win32_UI_Shell","Win32_UI_WindowsAndMessaging","Win32_Graphics_Gdi","Win32_System_Com","Win32_Media_Audio"] }` plus `raw-window-handle` for Windows, then update `Cargo.lock` (CI uses `--locked`).

## Caveats / Risks

- **AUMID versus shortcuts**: if the process sets an explicit AUMID, Microsoft says shortcuts must carry the same ID (<https://learn.microsoft.com/en-us/windows/win32/shell/appids>). The NSIS shortcuts have none (`native-installer.nsi:58-59`), so a pinned "Monoptah Native" shortcut and the running window may show as **separate taskbar buttons**, and the overlay or flash lands on the running one. Fixing that needs AUMID-on-.lnk in NSIS (System-plugin IPropertyStore or a plugin) plus an update path for existing shortcuts.
- **Dev builds**: `native:dev` has no shortcut. The registry AUMID works, but if dev and installed use the same AUMID they share notification settings, the toast display name and Action Center grouping. GPUI writes the HKCU key on the first toast, and the uninstaller does not remove `Software\Classes\AppUserModelId\...` (`native-installer.nsi:82-92`).
- **Toast icon**: GPUI writes only `DisplayName` with no `IconUri`, so the toast header icon may be generic.
- **Action Center persistence and clicks after the popup**: toasts normally persist in Action Center. Whether a click from Action Center (popup gone, process still alive) fires `Activated` without a COM activator, and what happens after the process exits, is **not verified here** and needs a manual Win11 test.
- `active_toasts` in GPUI is never pruned except by tag replacement or dismiss. The macOS tags include a sequence number, so each toast is a new entry.
- **CI**: Windows runs only `cargo check --all-targets` for monocode-gpui (`.github/workflows/native.yml:291-307`). There is no clippy and no tests on Windows, so `cfg(windows)` code gets type-checked but not linted, and Win32 behavior is untested. Linux clippy `-D warnings` will flag new dead code unless the `cfg_attr(not(macos), allow(dead_code))` attributes (`platform.rs:9`, `delivery.rs:85-87`) become `not(any(macos, windows))`. Pure logic (tag parsing, badge text, icon pixel buffer) can be unit-tested portably.
- `SetOverlayIcon` is ignored with small taskbar icons or when "Show badges on taskbar apps" is off, and it is lost on an Explorer restart without `TaskbarButtonCreated` handling.
- No web search tool was available. External facts come from Microsoft Learn pages fetched directly with curl, plus crate sources in `~/.cargo/registry`.
