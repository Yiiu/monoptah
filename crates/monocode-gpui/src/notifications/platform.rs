//! OS bridge. Call initialization and Dock updates on the UI thread. All other
//! results are channels so authorization/dispatch never block the UI thread.

use std::sync::mpsc::{self, Receiver};

use gpui::{App, Window};

use super::delivery::{Permission, Target};

#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
pub struct Notification {
    pub tag: String,
    pub subtitle: String,
    pub body: String,
    pub sound: bool,
}

#[cfg(target_os = "macos")]
#[path = "platform_macos.rs"]
mod implementation;

#[cfg(windows)]
#[path = "platform_windows.rs"]
mod implementation;

#[cfg(windows)]
pub use implementation::flash_taskbar;
#[cfg(any(target_os = "macos", windows))]
pub use implementation::{
    drain_clicks, open_settings, permission, request_permission, set_dock_badge,
};

pub fn initialize(window: &Window, cx: &App) -> bool {
    #[cfg(windows)]
    {
        implementation::initialize(window, cx)
    }
    #[cfg(not(windows))]
    let _ = (window, cx);
    #[cfg(target_os = "macos")]
    {
        implementation::initialize()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    false
}

pub fn show(notification: Notification, cx: &App) -> Receiver<Result<(), String>> {
    #[cfg(windows)]
    {
        implementation::show(notification, cx)
    }
    #[cfg(not(windows))]
    let _ = cx;
    #[cfg(target_os = "macos")]
    {
        implementation::show(notification)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = notification;
        resolved(Err(
            "Native notifications are unavailable on this platform".into()
        ))
    }
}

pub fn dismiss(tag: &str, cx: &App) {
    #[cfg(windows)]
    cx.dismiss_system_notification(tag);
    #[cfg(target_os = "macos")]
    implementation::dismiss(tag);
    #[cfg(not(windows))]
    let _ = (tag, cx);
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn permission() -> Receiver<Permission> {
    resolved(Permission::Unsupported)
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn request_permission() -> Receiver<Permission> {
    permission()
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn drain_clicks() -> Vec<Target> {
    Vec::new()
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn set_dock_badge(_: usize) {}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn open_settings() -> Result<(), String> {
    Err("Notification settings are unavailable on this platform".into())
}

fn resolved<T>(value: T) -> Receiver<T> {
    let (sender, receiver) = mpsc::channel();
    let _ = sender.send(value);
    receiver
}
