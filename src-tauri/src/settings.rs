use std::fs;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// 应用设置，持久化在 app_config_dir()/settings.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 截屏全局快捷键
    pub capture_hotkey: String,
    /// 显隐所有贴图的快捷键
    pub pins_hotkey: String,
    /// 开机自启
    pub autostart: bool,
    /// 历史截图保留数量
    pub history_limit: usize,
    /// 截图时隐藏任务栏（保证覆盖层完整盖住整个屏幕）
    pub hide_taskbar: bool,
    /// 截图时临时隐藏所有贴图（默认否：贴图与其它窗口一样入镜，与 Snipaste 一致）
    pub hide_pins_on_capture: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            capture_hotkey: "F1".into(),
            pins_hotkey: "F3".into(),
            autostart: false,
            history_limit: 50,
            hide_taskbar: true,
            hide_pins_on_capture: false,
        }
    }
}

fn settings_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .expect("无法解析应用配置目录")
        .join("settings.json")
}

pub fn load(app: &AppHandle) -> Settings {
    let path = settings_path(app);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app);
    eprintln!("[shotx] settings.json 写入 → {}", path.display());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // 原子写：先写临时文件再重命名
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    drop(f);
    let _ = fs::remove_file(&path);
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}
