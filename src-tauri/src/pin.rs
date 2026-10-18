//! 贴图（Pin）窗口：无边框、置顶、可拖、可缩放、整窗透明度。

use std::sync::atomic::Ordering;

use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl};

use crate::state::{AppState, PinEntry};
use crate::winutil;

/// 贴图数量上限，防止误触刷屏
const MAX_PINS: usize = 30;
/// 缩放上下限
const MIN_SCALE: f64 = 0.05;
const MAX_SCALE: f64 = 16.0;

/// 在桌面物理坐标 (x, y) 处创建贴图窗口，图像为 PNG 字节。
/// 窗口先隐藏，前端解码渲染完成后调用 pin_ready 再显示（避免白框一闪）。
pub fn create_pin(
    app: &AppHandle,
    png: Vec<u8>,
    base_w: u32,
    base_h: u32,
    x: i32,
    y: i32,
) -> Result<String, String> {
    let st = app.state::<AppState>();
    {
        let pins = st.pins.lock().unwrap();
        if pins.len() >= MAX_PINS {
            return Err(format!("贴图数量已达上限（{MAX_PINS}）"));
        }
    }
    let id = st.pin_counter.fetch_add(1, Ordering::Relaxed);
    let label = format!("pin-{id}");

    st.pins.lock().unwrap().insert(
        label.clone(),
        PinEntry { png, base_w, base_h, scale: 1.0 },
    );

    let url = format!("index.html?view=pin&id={label}");
    let w = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title("ShotX 贴图")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .closable(false)
        .shadow(true)
        .visible(false)
        .focused(false)
        .inner_size(base_w as f64, base_h as f64)
        .build()
        .map_err(|e| format!("创建贴图窗口失败: {e}"))?;
    // 物理像素精确覆盖选区（inner_size 是逻辑值，构建后立即用物理值校正）
    let _ = w.set_size(PhysicalSize::new(base_w, base_h));
    let _ = w.set_position(PhysicalPosition::new(x, y));
    Ok(label)
}

/// 前端完成渲染后的显示回执
pub fn pin_ready(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 缩放贴图：以窗口中心为锚点。返回实际生效的倍率（前端显示百分比）
pub fn set_scale(app: &AppHandle, label: &str, target_scale: f64) -> Result<f64, String> {
    let st = app.state::<AppState>();
    let (base_w, base_h, new_scale) = {
        let mut pins = st.pins.lock().unwrap();
        let entry = pins.get_mut(label).ok_or("贴图不存在")?;
        let s = target_scale.clamp(MIN_SCALE, MAX_SCALE);
        entry.scale = s;
        (entry.base_w, entry.base_h, s)
    };
    let w = app.get_webview_window(label).ok_or("贴图窗口已关闭")?;

    let nw = ((base_w as f64 * new_scale).round().max(16.0)) as u32;
    let nh = ((base_h as f64 * new_scale).round().max(16.0)) as u32;
    // 中心锚定
    if let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) {
        let dx = (size.width as i32 - nw as i32) / 2;
        let dy = (size.height as i32 - nh as i32) / 2;
        let _ = w.set_size(PhysicalSize::new(nw, nh));
        let _ = w.set_position(PhysicalPosition::new(pos.x + dx, pos.y + dy));
    } else {
        let _ = w.set_size(PhysicalSize::new(nw, nh));
    }
    Ok(new_scale)
}

/// 整窗透明度（分层窗口属性），前端滑条调用
pub fn set_opacity(app: &AppHandle, label: &str, opacity: f64) -> Result<(), String> {
    let w = app.get_webview_window(label).ok_or("贴图窗口已关闭")?;
    let hwnd = w.hwnd().map_err(|e| e.to_string())?;
    // hwnd.0 的内部表示随 windows 版本变化，统一转 isize
    let ok = winutil::set_window_alpha(hwnd.0 as isize, opacity);
    if !ok {
        return Err("设置透明度失败".into());
    }
    Ok(())
}

/// F3：整体隐藏/显示所有贴图
pub fn toggle_all(app: &AppHandle) {
    let st = app.state::<AppState>();
    let visible = st.pins_visible.load(Ordering::Acquire);
    for (label, w) in app.webview_windows() {
        if label.starts_with("pin-") {
            if visible {
                let _ = w.hide();
            } else {
                let _ = w.show();
            }
        }
    }
    st.pins_visible.store(!visible, Ordering::Release);
}

pub fn destroy(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.destroy();
    }
    // Destroyed 事件里会同步清理注册表
}

/// 贴图当前缩放（前端初始化时读取）
pub fn get_scale(app: &AppHandle, label: &str) -> Option<f64> {
    app.state::<AppState>()
        .pins
        .lock()
        .unwrap()
        .get(label)
        .map(|e| e.scale)
}
