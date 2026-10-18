//! 系统托盘：ShotX 唯一的常驻入口。

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::overlay;
use crate::pin;

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let capture_item = MenuItem::with_id(app, "capture", "截图", true, None::<&str>)?;
    let pins_item = MenuItem::with_id(app, "pins", "隐藏/显示所有贴图", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "设置与历史", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出 ShotX", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[&capture_item, &sep1, &pins_item, &settings_item, &sep2, &quit_item],
    )?;

    let settings = app.state::<crate::state::AppState>().settings.read().unwrap().clone();
    let tooltip = format!("ShotX — {} 截图", settings.capture_hotkey);

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().cloned().unwrap_or_else(|| {
            tauri::include_image!("icons/32x32.png")
        }))
        .tooltip(&tooltip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|tray, event| match event.id.as_ref() {
            "capture" => overlay::trigger_capture(tray.app_handle()),
            "pins" => pin::toggle_all(tray.app_handle()),
            "settings" => crate::winctl::open_settings(tray.app_handle()),
            "quit" => tray.app_handle().exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击托盘图标 = 直接截图
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                overlay::trigger_capture(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}
