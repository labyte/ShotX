//! 常规窗口（设置/历史）的按需创建与复用。

use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl};

/// 打开设置与历史窗口；已存在则复用（显示 + 聚焦）
pub fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        // 通知前端刷新列表（如快捷键变更后重开）
        let _ = app.emit_to("settings", "shotx://settings-refresh", ());
        return;
    }
    // SHOTX_DEBUG_PROBE=1：URL 附加 probe 参数，前端挂载后自动执行一次保存
    // （供自动化验证 update_settings 链路，不依赖像素级点击）
    let url = if std::env::var_os("SHOTX_DEBUG_PROBE").is_some() {
        "index.html?view=settings&probe=1"
    } else {
        "index.html?view=settings"
    };
    let builder = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App(url.into()))
    .title("ShotX 设置与历史")
    .inner_size(920.0, 640.0)
    .min_inner_size(700.0, 480.0)
    .resizable(true)
    .always_on_top(false)
    .visible(true)
    .focused(true);
    if let Err(e) = builder.build() {
        eprintln!("[shotx] 打开设置窗口失败: {e}");
    }
}

