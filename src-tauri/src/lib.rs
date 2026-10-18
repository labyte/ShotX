//! ShotX 应用装配：插件、托盘、全局快捷键、窗口事件。
//! 应用全程托盘驻留，启动不创建任何可见窗口。

pub mod capture;
mod commands;
pub mod diag;
mod history;
mod hotkey;
mod overlay;
mod pin;
mod settings;
mod state;
mod tray;
mod winctl;
mod winutil;

use tauri::{Manager, WindowEvent};

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 尽早声明 Per-Monitor-V2 DPI 感知：保证 DXGI/GDI 坐标与物理像素一致
    #[cfg(windows)]
    winutil::set_dpi_awareness();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {
            // 重复启动直接退出（托盘已在）；二次实例不做任何事
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        // 贴图原生菜单动作 → 转发回对应贴图窗口执行（id 格式 "action:label"）
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_string();
            if let Some((action, label)) = id.split_once(':') {
                use tauri::Emitter;
                let _ = app.emit_to(label, "pin-menu-action", action.to_string());
            }
        })
        .on_window_event(|window, event| {
            // 贴图窗口销毁时清理注册表
            if let WindowEvent::Destroyed = event {
                let label = window.label();
                if label.starts_with("pin-") {
                    if let Some(st) = window.app_handle().try_state::<AppState>() {
                        st.pins.lock().unwrap().remove(label);
                    }
                }
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let loaded = settings::load(&handle);
            *handle.state::<AppState>().settings.write().unwrap() = loaded.clone();

            // 历史目录
            let hist_dir = handle.path().app_data_dir()?.join("history");
            *handle.state::<AppState>().history.lock().unwrap() =
                Some(history::HistoryStore::open(hist_dir));

            hotkey::apply(&handle);
            tray::build(&handle)?;

            // 调试开关：启动即打开设置窗口（验证 JS/IPC 管道）
            if std::env::var_os("SHOTX_DEBUG_WINDOW").is_some() {
                winctl::open_settings(&handle);
            }

            // 开机自启状态对齐（以设置文件为准）
            if loaded.autostart {
                use tauri_plugin_autostart::ManagerExt;
                let _ = handle.autolaunch().enable();
            }

            // 后台预热 DXGI 引擎，缩短首次截图延迟
            std::thread::spawn(|| {
                match capture::capture_monitors() {
                    Ok(shots) => {
                        eprintln!("[shotx] 捕获预热完成：{} 个显示器，后端可用", shots.len());
                        // 调试开关：把预热帧落盘，用于检查画面内容是否被环境置黑
                        if std::env::var_os("SHOTX_DUMP").is_some() {
                            for (m, f) in &shots {
                                let path = std::env::temp_dir().join(format!(
                                    "shotx-dump-{}.png",
                                    m.index
                                ));
                                let img = image::RgbaImage::from_raw(f.width, f.height, f.rgba.clone());
                                match img {
                                    Some(img) => {
                                        if img.save(&path).is_ok() {
                                            eprintln!("[shotx] 预热帧已落盘 {}", path.display());
                                        }
                                    }
                                    None => eprintln!("[shotx] 预热帧缓冲长度异常"),
                                }
                            }
                        }
                    }
                    Err(e) => crate::diag::log(&format!("捕获预热失败: {e}")),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::debug_log,
            commands::take_capture_frame,
            commands::overlay_ready,
            commands::overlay_ack,
            commands::overlay_finish,
            commands::finish_capture,
            commands::copy_png,
            commands::write_png,
            commands::get_pin_payload,
            commands::pin_ready,
            commands::pin_set_scale,
            commands::pin_get_scale,
            commands::pin_set_opacity,
            commands::pin_menu,
            commands::pin_close,
            commands::history_list,
            commands::history_thumb,
            commands::history_file,
            commands::history_pin,
            commands::history_delete,
            commands::history_clear,
            commands::get_settings,
            commands::update_settings,
            commands::open_settings_window,
            commands::hide_self,
            commands::refresh_tray_tooltip,
        ]);

    let app = builder
        .build(tauri::generate_context!())
        .expect("ShotX 启动失败");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            // 兜底恢复：截图会话中退出不能把任务栏留在隐藏状态
            overlay::force_cleanup(app_handle);
        }
    });
}
