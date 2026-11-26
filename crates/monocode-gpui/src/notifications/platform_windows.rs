//! Windows toast delivery and taskbar attention, called on the GPUI thread.

use std::{
    cell::{Cell, RefCell},
    mem,
    sync::mpsc::Receiver,
};

use gpui::{App, SystemNotification, SystemNotificationAction, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
            DeleteObject,
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        UI::{
            Shell::{ITaskbarList3, ShellExecuteW, TaskbarList},
            WindowsAndMessaging::{
                CreateIconIndirect, DestroyIcon, FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO,
                FlashWindowEx, HICON, ICONINFO, SW_SHOWNORMAL,
            },
        },
    },
    core::{BOOL, Error, HSTRING, PCWSTR, w},
};

use super::{Notification, Permission, Target, resolved};
use crate::notifications::badge::{BADGE_SIDE, BadgePixels, badge_pixels};
use crate::{sound, theme};

const OPEN: &str = "open";

thread_local! {
    static WINDOW: Cell<Option<HWND>> = const { Cell::new(None) };
    static CLICKS: RefCell<Vec<Target>> = const { RefCell::new(Vec::new()) };
}

pub fn initialize(window: &Window, cx: &App) -> bool {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    WINDOW.set(Some(HWND(handle.hwnd.get() as *mut _)));
    cx.on_system_notification_response(|response, _| {
        if response.action_id.as_deref().is_none_or(|id| id == OPEN)
            && let Some(target) = Target::from_tag(&response.tag)
        {
            CLICKS.with_borrow_mut(|clicks| clicks.push(target));
        }
    });
    true
}

pub fn permission() -> Receiver<Permission> {
    resolved(Permission::Granted)
}

pub fn request_permission() -> Receiver<Permission> {
    permission()
}

pub fn show(notification: Notification, cx: &App) -> Receiver<Result<(), String>> {
    let Some(target) = Target::from_tag(&notification.tag) else {
        return resolved(Err("This notification has no Windows target".into()));
    };
    cx.show_system_notification(SystemNotification {
        tag: notification.tag.into(),
        title: notification.subtitle.into(),
        body: notification.body.into(),
        actions: vec![SystemNotificationAction {
            id: OPEN.into(),
            label: "Open".into(),
        }],
    });
    // GPUI makes Windows toasts silent. The app retains its existing cue.
    if notification.sound && matches!(target, Target::Session(_)) {
        sound::play(sound::Cue::Input);
    }
    resolved(Ok(()))
}

pub fn drain_clicks() -> Vec<Target> {
    CLICKS.with_borrow_mut(mem::take)
}

pub fn open_settings() -> Result<(), String> {
    // SAFETY: The verb and URI are static, NUL-terminated UTF-16 strings.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("ms-settings:notifications"),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    let code = result.0 as isize;
    if code <= 32 {
        return Err(format!(
            "Could not open Windows notification settings ({code})"
        ));
    }
    Ok(())
}

pub fn flash_taskbar() {
    if let Some(hwnd) = WINDOW.get() {
        let info = FLASHWINFO {
            cbSize: mem::size_of::<FLASHWINFO>() as u32,
            hwnd,
            dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
            uCount: 0,
            dwTimeout: 0,
        };
        // SAFETY: The HWND belongs to the GPUI shell on this thread.
        let _ = unsafe { FlashWindowEx(&info) };
    }
}

pub fn set_dock_badge(count: usize) {
    if let Some(hwnd) = WINDOW.get()
        && let Err(error) = set_overlay_icon(hwnd, count)
    {
        log::debug!("Could not update Windows taskbar badge: {error}");
    }
}

fn set_overlay_icon(hwnd: HWND, count: usize) -> windows::core::Result<()> {
    // SAFETY: GPUI initializes OLE on the UI thread. The taskbar copies the icon.
    unsafe {
        let taskbar: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)?;
        taskbar.HrInit()?;
        if count == 0 {
            return taskbar.SetOverlayIcon(hwnd, HICON::default(), PCWSTR::null());
        }

        let ink = if theme::is_light() {
            theme::white(1.)
        } else {
            theme::black(1.)
        };
        let pixels = badge_pixels(count, rgba_bytes(theme::warning_color()), rgba_bytes(ink));
        let icon = create_icon(pixels)?;
        let description = HSTRING::from(format!("Sessions that need you: {count}"));
        let result = taskbar.SetOverlayIcon(hwnd, icon, &description);
        let _ = DestroyIcon(icon);
        result
    }
}

fn rgba_bytes(color: gpui::Hsla) -> [u8; 4] {
    let gpui::Rgba { r, g, b, a } = color.into();
    [r, g, b, a].map(|component| (component * 255.).round() as u8)
}

fn create_icon(pixels: BadgePixels) -> windows::core::Result<HICON> {
    let side = BADGE_SIDE as i32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: side,
            biHeight: -side, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    // SAFETY: The DIB owns exactly one BGRA pixel per input pixel. CreateIconIndirect
    // copies both bitmaps, which are deleted before returning.
    unsafe {
        let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let out = std::slice::from_raw_parts_mut(bits.cast::<[u8; 4]>(), pixels.len());
        for (out, [r, g, b, a]) in out.iter_mut().zip(pixels) {
            *out = [b, g, r, a];
        }
        // A 32-bit icon uses the alpha channel. Supply the required mono mask
        // with a zeroed, word-aligned scanline for each row.
        let mask_bits = [0u8; BADGE_SIDE * 2];
        let mask = CreateBitmap(side, side, 1, 1, Some(mask_bits.as_ptr().cast()));
        let icon = if mask.is_invalid() {
            Err(Error::from_thread())
        } else {
            let result = CreateIconIndirect(&ICONINFO {
                fIcon: BOOL(1),
                hbmMask: mask,
                hbmColor: color,
                ..Default::default()
            });
            let _ = DeleteObject(mask.into());
            result
        };
        let _ = DeleteObject(color.into());
        icon
    }
}
