//! 全局快捷键注册与热更新。

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::overlay;
use crate::pin;

/// 按当前设置重新注册全部快捷键（启动时与设置变更后调用）
pub fn apply(app: &AppHandle) {
    let st = app.state::<crate::state::AppState>();
    let settings = st.settings.read().unwrap().clone();
    let gs = app.global_shortcut();
    if let Err(e) = gs.unregister_all() {
        eprintln!("[shotx] 注销快捷键失败: {e}");
    }

    register_one(app, &settings.capture_hotkey, |app| overlay::trigger_capture(app));
    register_one(app, &settings.pins_hotkey, |app| pin::toggle_all(app));
}

fn register_one(app: &AppHandle, combo: &str, action: fn(&AppHandle)) {
    if combo.trim().is_empty() {
        return;
    }
    let parsed: Result<Shortcut, _> = combo.parse();
    match parsed {
        Ok(shortcut) => {
            let res = app.global_shortcut().on_shortcut(shortcut, move |_app, _gs, event| {
                if event.state == ShortcutState::Pressed {
                    action(_app);
                }
            });
            if let Err(e) = res {
                eprintln!("[shotx] 注册快捷键 {combo} 失败: {e}");
            }
        }
        Err(_) => eprintln!("[shotx] 快捷键组合无法解析: {combo}"),
    }
}
