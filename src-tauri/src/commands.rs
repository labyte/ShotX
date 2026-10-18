//! 前端可调用的命令集合。
//!
//! 大字节一律走 v2 类型化 IPC 的原始二进制通道：
//! - Rust → JS：命令返回 `tauri::ipc::Response`，前端拿到 `ArrayBuffer`
//! - JS → Rust：`invoke(cmd, uint8Array, { headers })`，Rust 侧用
//!   `ipc::Request` 读 `InvokeBody::Raw`
//! `finish_capture` 的元数据（动作/路径/位置）打包进同一份原始缓冲：
//! `[u32 元数据长度 LE][元数据 JSON][PNG 字节]`，避免 header 的非 ASCII 麻烦。

use std::borrow::Cow;
use std::fs;

use image::GenericImageView;
use serde::Deserialize;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::history::HistoryItem;
use crate::settings::Settings;
use crate::state::AppState;

/// 前端序列化用的历史条目（路径转字符串）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItemDto {
    pub id: String,
    pub created: u64,
    pub width: u32,
    pub height: u32,
    pub file: String,
}

fn history_items(st: &AppState) -> Vec<HistoryItemDto> {
    let guard = st.history.lock().unwrap();
    guard
        .as_ref()
        .map(|h| {
            h.list()
                .into_iter()
                .map(|it| HistoryItemDto {
                    id: it.id,
                    created: it.created,
                    width: it.width,
                    height: it.height,
                    file: it.file.display().to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 渲染端日志转发（调试用，落盘 %TEMP%\shotx-diag.log）
#[tauri::command]
pub fn debug_log(level: String, msg: String) {
    crate::diag::log(&format!("js:{level} {msg}"));
}

// ---------- 截图覆盖层 ----------

/// overlay 拉取本轮捕获帧：`[w u32 LE][h u32 LE][RGBA...]`；无帧则空缓冲
#[tauri::command]
pub fn take_capture_frame(app: AppHandle, monitor: u32) -> Response {
    let st = app.state::<AppState>();
    let g = st.frames.lock().unwrap();
    match g.get(&monitor) {
        Some(f) => {
            let mut v = Vec::with_capacity(8 + f.rgba.len());
            v.extend_from_slice(&f.width.to_le_bytes());
            v.extend_from_slice(&f.height.to_le_bytes());
            v.extend_from_slice(&f.rgba);
            Response::new(v)
        }
        None => Response::new(Vec::new()),
    }
}

/// overlay 前端挂载就绪（重发 frame-ready 兜底）
#[tauri::command]
pub fn overlay_ready(app: AppHandle, monitor: u32) {
    crate::overlay::note_ready(&app, monitor);
}

/// overlay 已渲染完成回执
#[tauri::command]
pub fn overlay_ack(app: AppHandle, monitor: u32) {
    eprintln!("[shotx] overlay-{monitor} 渲染回执");
    crate::overlay::note_ack(&app, monitor);
}

/// 结束截图会话（完成或取消）
#[tauri::command]
pub fn overlay_finish(app: AppHandle) {
    eprintln!("[shotx] overlay_finish 被前端调用");
    crate::overlay::finish(&app);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinishMeta {
    /// copy | save | pin
    action: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    monitor: Option<u32>,
    /// 选区左上角在该显示器内的物理坐标
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
}

/// 完成一次截图：入历史 + 按动作执行 复制/保存/贴图
#[tauri::command]
pub async fn finish_capture(app: AppHandle, request: Request<'_>) -> Result<Option<String>, String> {
    let body: Vec<u8> = match request.body() {
        InvokeBody::Raw(b) => b.to_vec(),
        _ => return Err("finish_capture 需要原始二进制请求体".into()),
    };
    if body.len() < 4 {
        return Err("请求体过短".into());
    }
    let meta_len = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
    if body.len() < 4 + meta_len {
        return Err("请求体不完整".into());
    }
    let meta: FinishMeta =
        serde_json::from_slice(&body[4..4 + meta_len]).map_err(|e| e.to_string())?;
    let png = &body[4 + meta_len..];
    if png.len() < 8 {
        return Err("缺少 PNG 数据".into());
    }

    // 1) 入历史（失败不阻断动作本身）
    {
        let st = app.state::<AppState>();
        let limit = st.settings.read().unwrap().history_limit;
        let mut g = st.history.lock().unwrap();
        if let Some(h) = g.as_mut() {
            if let Err(e) = h.push(png, limit) {
                eprintln!("[shotx] 历史保存失败: {e}");
            }
        }
    }

    // 2) 按动作执行
    match meta.action.as_str() {
        "copy" => copy_png_to_clipboard(png)?,
        "save" => {
            let path = meta.path.ok_or("保存缺少路径")?;
            fs::write(&path, png).map_err(|e| format!("写入文件失败: {e}"))?;
        }
        "pin" => {
            let (mi, x, y) = pin_position(&app, meta.monitor, meta.x, meta.y);
            let label = crate::pin::create_pin(&app, png.to_vec(), dims_width(png)?, dims_height(png)?, x, y)?;
            let _ = mi;
            return Ok(Some(label));
        }
        other => return Err(format!("未知动作: {other}")),
    }
    Ok(None)
}

/// 优先取最近捕获会话记录的显示器；若为空（应用刚启动未截过图），
/// 回退到系统主显示器，保证历史贴图总能落位。
fn known_monitors(app: &AppHandle) -> Vec<crate::capture::MonitorInfo> {
    let st = app.state::<AppState>();
    let monitors = st.monitors.lock().unwrap().clone();
    if !monitors.is_empty() {
        return monitors;
    }
    if let Ok(Some(m)) = app.primary_monitor() {
        let pos = m.position();
        let size = m.size();
        return vec![crate::capture::MonitorInfo {
            index: 0,
            name: m.name().map(|s| s.to_string()).unwrap_or_default(),
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            is_primary: true,
        }];
    }
    Vec::new()
}

fn pin_position(app: &AppHandle, monitor: Option<u32>, lx: i32, ly: i32) -> (Option<u32>, i32, i32) {
    let monitors = known_monitors(app);
    let m = match monitor {
        Some(idx) => monitors.iter().find(|m| m.index == idx).or_else(|| {
            monitors.iter().find(|m| m.is_primary).or(monitors.first())
        }),
        None => monitors
            .iter()
            .find(|m| m.is_primary)
            .or(monitors.first()),
    };
    match m {
        Some(m) => (Some(m.index), m.x + lx, m.y + ly),
        None => (None, lx, ly),
    }
}

fn dims_width(png: &[u8]) -> Result<u32, String> {
    image::load_from_memory(png)
        .map(|i| i.dimensions().0)
        .map_err(|e| e.to_string())
}
fn dims_height(png: &[u8]) -> Result<u32, String> {
    image::load_from_memory(png)
        .map(|i| i.dimensions().1)
        .map_err(|e| e.to_string())
}

fn copy_png_to_clipboard(png: &[u8]) -> Result<(), String> {
    let img = image::load_from_memory(png).map_err(|e| e.to_string())?;
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_image(arboard::ImageData {
        width: w as usize,
        height: h as usize,
        bytes: Cow::Owned(rgba.into_raw()),
    })
    .map_err(|e| e.to_string())
}

/// 仅复制 PNG 到剪贴板（贴图窗口右键菜单用，不写历史）
#[tauri::command]
pub async fn copy_png(request: Request<'_>) -> Result<(), String> {
    let png = match request.body() {
        InvokeBody::Raw(b) => b.to_vec(),
        _ => return Err("需要原始 PNG 数据".into()),
    };
    copy_png_to_clipboard(&png)
}

/// 将 PNG 写入指定路径（贴图"另存为"用）。请求体为打包容器：
/// [u32 meta 长度 LE][{"path":...}][PNG 字节]
#[tauri::command]
pub async fn write_png(request: Request<'_>) -> Result<(), String> {
    let body = match request.body() {
        InvokeBody::Raw(b) => b,
        _ => return Err("需要原始数据请求体".into()),
    };
    if body.len() < 4 {
        return Err("请求体过短".into());
    }
    let meta_len = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
    if body.len() < 4 + meta_len {
        return Err("请求体不完整".into());
    }
    #[derive(Deserialize)]
    struct M {
        path: String,
    }
    let meta: M = serde_json::from_slice(&body[4..4 + meta_len]).map_err(|e| e.to_string())?;
    fs::write(&meta.path, &body[4 + meta_len..]).map_err(|e| format!("写入文件失败: {e}"))
}

// ---------- 贴图 ----------

/// 弹出贴图的系统原生右键菜单（不受窗口尺寸限制，不会被裁剪）。
/// x/y 为窗口内 CSS 坐标；菜单动作以 "action:窗口label" 作为菜单项 id，
/// 由 lib.rs 的全局 on_menu_event 解析后转发回该贴图窗口执行。
#[tauri::command]
pub fn pin_menu(
    app: AppHandle,
    window: tauri::Window,
    x: f64,
    y: f64,
    ontop: bool,
) -> Result<(), String> {
    use tauri::menu::{ContextMenu, MenuBuilder, MenuItem, PredefinedMenuItem, Submenu};

    let label = window.label().to_string();
    let mid = |s: &str| format!("{s}:{label}");
    let mk = |id: &str, text: &str| {
        MenuItem::with_id(&app, id, text, true, None::<&str>).map_err(|e| e.to_string())
    };

    let copy = mk(&mid("copy"), "复制到剪贴板")?;
    let save = mk(&mid("save"), "另存为…")?;
    let sep1 = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;
    let op100 = mk(&mid("op1.00"), "100%")?;
    let op75 = mk(&mid("op0.75"), "75%")?;
    let op50 = mk(&mid("op0.50"), "50%")?;
    let op25 = mk(&mid("op0.25"), "25%")?;
    let opacity = Submenu::with_items(
        &app,
        "透明度",
        true,
        &[&op100, &op75, &op50, &op25],
    )
    .map_err(|e| e.to_string())?;
    let top = mk(&mid("top"), if ontop { "取消置顶" } else { "保持置顶" })?;
    let actual = mk(&mid("actual"), "恢复实际大小")?;
    let sep2 = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;
    let close = mk(&mid("close"), "关闭贴图 (Esc/双击)")?;

    let menu = MenuBuilder::new(&app)
        .item(&copy)
        .item(&save)
        .item(&sep1)
        .item(&opacity)
        .item(&top)
        .item(&actual)
        .item(&sep2)
        .item(&close)
        .build()
        .map_err(|e| e.to_string())?;

    let scale = window.scale_factor().unwrap_or(1.0);
    let pos = tauri::PhysicalPosition::new(x * scale, y * scale);
    menu.popup_at(window, pos).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_pin_payload(app: AppHandle, window: WebviewWindow) -> Response {
    let st = app.state::<AppState>();
    let g = st.pins.lock().unwrap();
    match g.get(window.label()) {
        Some(e) => Response::new(e.png.clone()),
        None => Response::new(Vec::new()),
    }
}

#[tauri::command]
pub fn pin_ready(app: AppHandle, window: WebviewWindow) {
    crate::pin::pin_ready(&app, window.label());
}

#[tauri::command]
pub fn pin_set_scale(app: AppHandle, window: WebviewWindow, scale: f64) -> Result<f64, String> {
    crate::pin::set_scale(&app, window.label(), scale)
}

#[tauri::command]
pub fn pin_get_scale(app: AppHandle, window: WebviewWindow) -> Option<f64> {
    crate::pin::get_scale(&app, window.label())
}

#[tauri::command]
pub fn pin_set_opacity(app: AppHandle, window: WebviewWindow, opacity: f64) -> Result<(), String> {
    crate::pin::set_opacity(&app, window.label(), opacity)
}

#[tauri::command]
pub fn pin_close(app: AppHandle, window: WebviewWindow) {
    crate::pin::destroy(&app, window.label());
}

// ---------- 历史 ----------

#[tauri::command]
pub fn history_list(app: AppHandle) -> Vec<HistoryItemDto> {
    let st = app.state::<AppState>();
    history_items(&st)
}

#[tauri::command]
pub fn history_thumb(app: AppHandle, id: String) -> Response {
    let st = app.state::<AppState>();
    let g = st.history.lock().unwrap();
    let bytes = g
        .as_ref()
        .and_then(|h| h.item(&id))
        .and_then(|it| fs::read(&it.thumb).ok())
        .unwrap_or_default();
    Response::new(bytes)
}

/// 从历史贴图：默认放在主显中心
#[tauri::command]
pub fn history_pin(app: AppHandle, id: String) -> Result<String, String> {
    let st = app.state::<AppState>();
    let (png, w, h) = {
        let g = st.history.lock().unwrap();
        let it: Option<HistoryItem> = g.as_ref().and_then(|h| h.item(&id));
        let it = it.ok_or("历史条目不存在")?;
        let png = fs::read(&it.file).map_err(|e| e.to_string())?;
        (png, it.width, it.height)
    };
    let monitors = known_monitors(&app);
    let m = monitors
        .iter()
        .find(|m| m.is_primary)
        .or_else(|| monitors.first())
        .ok_or("未找到显示器")?;
    let x = m.x + (m.width.saturating_sub(w) / 2) as i32;
    let y = m.y + (m.height.saturating_sub(h) / 2) as i32;
    crate::pin::create_pin(&app, png, w, h, x, y)
}

/// 历史原图字节（复制/另存用）
#[tauri::command]
pub fn history_file(app: AppHandle, id: String) -> Response {
    let st = app.state::<AppState>();
    let g = st.history.lock().unwrap();
    let bytes = g
        .as_ref()
        .and_then(|h| h.item(&id))
        .and_then(|it| fs::read(&it.file).ok())
        .unwrap_or_default();
    Response::new(bytes)
}

#[tauri::command]
pub fn history_delete(app: AppHandle, id: String) -> Result<(), String> {
    let st = app.state::<AppState>();
    let mut g = st.history.lock().unwrap();
    match g.as_mut() {
        Some(h) => h.remove(&id),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn history_clear(app: AppHandle) -> Result<(), String> {
    let st = app.state::<AppState>();
    let mut g = st.history.lock().unwrap();
    match g.as_mut() {
        Some(h) => h.clear(),
        None => Ok(()),
    }
}

// ---------- 设置 ----------

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    app.state::<AppState>().settings.read().unwrap().clone()
}

#[tauri::command]
pub fn update_settings(app: AppHandle, new_settings: Settings) -> Result<Settings, String> {
    eprintln!("[shotx] 应用新设置: {new_settings:?}");
    crate::settings::save(&app, &new_settings).map_err(|e| e.to_string())?;
    {
        let st = app.state::<AppState>();
        *st.settings.write().unwrap() = new_settings.clone();
        // 历史保留数变化
        let mut g = st.history.lock().unwrap();
        if let Some(h) = g.as_mut() {
            h.apply_limit(new_settings.history_limit)?;
        }
    }
    // 快捷键热更新
    crate::hotkey::apply(&app);
    // 开机自启
    {
        use tauri_plugin_autostart::ManagerExt;
        let al = app.autolaunch();
        let _ = if new_settings.autostart { al.enable() } else { al.disable() };
    }
    Ok(new_settings)
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle) {
    crate::winctl::open_settings(&app);
}

/// 隐藏指定窗口（设置窗口的"关闭"语义）
#[tauri::command]
pub fn hide_self(app: AppHandle, window: WebviewWindow) {
    let _ = window.hide();
    // 贴图与覆盖层之外的关闭请求一律走这里；贴图销毁有 pin_close
    let _ = app;
}

/// 刷新托盘 tooltip（快捷键变更后）
#[tauri::command]
pub fn refresh_tray_tooltip(app: AppHandle) {
    let st = app.state::<AppState>();
    let hk = st.settings.read().unwrap().capture_hotkey.clone();
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_tooltip(Some(format!("ShotX — {hk} 截图")));
    }
}
