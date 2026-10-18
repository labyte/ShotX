//! 截图覆盖层：会话状态机与每显示器窗口管理。
//!
//! 时序：热键 → 隐藏贴图(可选) → 捕获全部屏幕（此刻本应用所有窗口不可见，
//! 不会自捕获）→ 创建/复用隐藏 overlay 窗口 → 通知各窗口取帧 → 各窗口
//! 渲染完成后回执 → 全部就绪再统一显示并聚焦 → 用户完成/取消 → 隐藏窗口、
//! 恢复任务栏与贴图。

use std::time::Duration;

use serde::Serialize;
use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl};

use crate::capture::{self, MonitorInfo, RawFrame};
use crate::state::AppState;
use crate::winutil;

/// frame-ready 事件负载
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameReady {
    pub seq: u64,
    pub monitor: u32,
    pub width: u32,
    pub height: u32,
    pub monitor_x: i32,
    pub monitor_y: i32,
    pub is_primary: bool,
}

/// 一次截图会话的入口（热键 / 托盘调用）
pub fn trigger_capture(app: &AppHandle) {
    let st = app.state::<AppState>();
    if st.capture_active.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return; // 已在会话中
    }
    if let Err(e) = do_capture(app, &st) {
        crate::diag::log(&format!("截图失败: {e}"));
        // 回滚
        restore_pins(app, &st);
        winutil::restore_taskbars();
        st.capture_active.store(false, std::sync::atomic::Ordering::Release);
    }
}

fn do_capture(app: &AppHandle, st: &AppState) -> Result<(), String> {
    let settings = st.settings.read().unwrap().clone();

    // 0) 可选：把自家贴图先藏掉，避免入镜
    if settings.hide_pins_on_capture {
        let mut hidden = st.pins_hidden_by_capture.lock().unwrap();
        hidden.clear();
        for (label, w) in app.webview_windows() {
            if label.starts_with("pin-") && w.is_visible().unwrap_or(false) {
                let _ = w.hide();
                hidden.push(label);
            }
        }
    }

    // 1) 底层捕获所有显示器
    let shots = capture::capture_monitors()?;

    // 2) 会话数据入状态
    let seq = st.capture_seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    {
        let mut frames = st.frames.lock().unwrap();
        frames.clear();
        for (m, f) in &shots {
            frames.insert(
                m.index,
                crate::state::CapturedFrame {
                    width: f.width,
                    height: f.height,
                    rgba: f.rgba.clone(),
                },
            );
        }
        *st.monitors.lock().unwrap() = shots.iter().map(|(m, _)| m.clone()).collect();
        st.overlay_acks.lock().unwrap().clear();
    }
    st.overlay_expected
        .store(shots.len() as u32, std::sync::atomic::Ordering::Release);

    // 3) 覆盖层窗口对齐显示器几何
    ensure_overlay_windows(app, &shots)?;

    // 4) 隐藏任务栏（配置开关），保证覆盖层可盖住整屏
    if settings.hide_taskbar {
        winutil::hide_taskbars();
    }

    // 5) 通知各 overlay 拉帧；渲染回执见 note_ack
    for (m, f) in &shots {
        let payload = FrameReady {
            seq,
            monitor: m.index,
            width: f.width,
            height: f.height,
            monitor_x: m.x,
            monitor_y: m.y,
            is_primary: m.is_primary,
        };
        let _ = app.emit_to(format!("overlay-{}", m.index), "shotx://frame-ready", payload);
    }

    // 6) 看门狗：4s 内无任何窗口就绪则终止会话（首次 WebView2 冷启动较慢，
    //    就绪者会即时自显，无需等齐）
    let app2 = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(4000));
        session_watchdog(&app2, seq);
    });
    Ok(())
}

/// overlay 前端挂载完成时主动调用：若此刻有活动会话（事件可能因 JS 尚未
/// 就绪而错过），重发 frame-ready 事件，保证握手可靠。
pub fn note_ready(app: &AppHandle, monitor: u32) {
    let st = app.state::<AppState>();
    if !st.capture_active.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let seq = st.capture_seq.load(std::sync::atomic::Ordering::SeqCst);
    let monitors = st.monitors.lock().unwrap().clone();
    let Some(m) = monitors.iter().find(|m| m.index == monitor) else {
        return;
    };
    let (width, height) = match st.frames.lock().unwrap().get(&monitor) {
        Some(f) => (f.width, f.height),
        None => return,
    };
    eprintln!("[shotx] overlay-{monitor} 就绪握手，补发 frame-ready (seq={seq})");
    let payload = FrameReady {
        seq,
        monitor,
        width,
        height,
        monitor_x: m.x,
        monitor_y: m.y,
        is_primary: m.is_primary,
    };
    let _ = app.emit_to(format!("overlay-{monitor}"), "shotx://frame-ready", payload);
}

/// overlay 前端渲染完成回执：就绪即显示该窗口（每显示器独立，
/// 消除"等齐统一显示"的时序耦合；未就绪的窗口永不显示 → 任何
/// 故障下用户都不会看到黑色等待层，最坏情况是"按了没反应"）。
pub fn note_ack(app: &AppHandle, monitor: u32) {
    use std::sync::atomic::Ordering::{Acquire, SeqCst};
    let st = app.state::<AppState>();
    if !st.capture_active.load(Acquire) {
        return;
    }
    let seq = st.capture_seq.load(SeqCst);
    st.overlay_acks.lock().unwrap().insert((seq, monitor));

    let Some(w) = app.get_webview_window(&format!("overlay-{monitor}")) else {
        return;
    };
    let first = st.shown_seq.swap(seq, SeqCst) != seq;
    let _ = w.show();
    if !first {
        return;
    }
    // 本轮首个就绪：聚焦光标所在屏的就绪窗口（不在/未就绪则聚焦自己）
    let cursor = app
        .cursor_position()
        .map(|p| (p.x as i32, p.y as i32))
        .unwrap_or((i32::MIN / 2, i32::MIN / 2));
    let focus_monitor = {
        let acks = st.overlay_acks.lock().unwrap();
        let monitors = st.monitors.lock().unwrap();
        monitors
            .iter()
            .find(|m| {
                cursor.0 >= m.x
                    && cursor.0 < m.x + m.width as i32
                    && cursor.1 >= m.y
                    && cursor.1 < m.y + m.height as i32
                    && acks.contains(&(seq, m.index))
            })
            .map(|m| m.index)
    };
    let label = format!("overlay-{}", focus_monitor.unwrap_or(monitor));
    if let Some(f) = app.get_webview_window(&label) {
        let _ = f.set_focus();
    }
    // 截图已包含贴图，恢复显示
    restore_pins(app, &st);
}

/// 会话看门狗：超时仍无任何窗口就绪（JS 未加载/帧管道故障）→ 终止会话，
/// 恢复任务栏，避免残留半启动状态。有任一窗口就绪则不干预（就绪者已自显）。
fn session_watchdog(app: &AppHandle, seq: u64) {
    use std::sync::atomic::Ordering::{Acquire, SeqCst};
    let st = app.state::<AppState>();
    if !st.capture_active.load(Acquire) {
        return;
    }
    if st.capture_seq.load(SeqCst) != seq {
        return; // 已是更新的会话
    }
    let any_acked = st
        .overlay_acks
        .lock()
        .unwrap()
        .iter()
        .any(|(s, _)| *s == seq);
    if !any_acked {
        crate::diag::log(&format!("覆盖层 {seq} 4s 内无一就绪（JS 未加载？），终止会话"));
        finish(app);
    }
}

fn restore_pins(app: &AppHandle, st: &AppState) {
    let labels: Vec<String> = st.pins_hidden_by_capture.lock().unwrap().drain(..).collect();
    for label in labels {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.show();
        }
    }
}

/// 创建/对齐每显示器覆盖层窗口，并清理多余窗口
fn ensure_overlay_windows(app: &AppHandle, shots: &[(MonitorInfo, RawFrame)]) -> Result<(), String> {
    let valid: Vec<String> = shots
        .iter()
        .map(|(m, _)| format!("overlay-{}", m.index))
        .collect();

    // 关闭已不存在的显示器对应的窗口
    for (label, w) in app.webview_windows() {
        if label.starts_with("overlay-") && !valid.contains(&label) {
            let _ = w.destroy();
        }
    }

    for (m, f) in shots {
        let label = format!("overlay-{}", m.index);
        if let Some(w) = app.get_webview_window(&label) {
            // 显示器可能被热插拔/改分辨率，几何对齐
            let _ = w.set_size(PhysicalSize::new(f.width, f.height));
            let _ = w.set_position(PhysicalPosition::new(m.x, m.y));
        } else {
            let url = format!("index.html?view=overlay&m={}", m.index);
            let w = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
                .title("ShotX 截图")
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .maximizable(false)
                .minimizable(false)
                .closable(false)
                .shadow(false)
                .visible(false)
                .focused(false)
                .inner_size(100.0, 100.0)
                .build()
                .map_err(|e| format!("创建覆盖层失败: {e}"))?;
            let _ = w.set_size(PhysicalSize::new(f.width, f.height));
            let _ = w.set_position(PhysicalPosition::new(m.x, m.y));
        }
    }
    Ok(())
}

/// 结束截图会话（复制/保存/贴图完成后或取消）
pub fn finish(app: &AppHandle) {
    let st = app.state::<AppState>();
    for (label, w) in app.webview_windows() {
        if label.starts_with("overlay-") {
            let _ = w.hide();
        }
    }
    winutil::restore_taskbars();
    // 若贴图曾被"截图时隐藏贴图"临时藏起，此处必须恢复
    // （reveal 未发生就被取消的路径也要走到这里）
    restore_pins(app, &st);
    st.frames.lock().unwrap().clear();
    st.overlay_acks.lock().unwrap().clear();
    st.capture_active
        .store(false, std::sync::atomic::Ordering::Release);
}

/// 会话仍活动但用户切换了焦点窗口丢失时，允许 Esc 生效：
/// 由前端全局 keydown 兜底，无需 Rust 侧处理。
pub fn force_cleanup(app: &AppHandle) {
    if app.state::<AppState>().capture_active.load(std::sync::atomic::Ordering::Acquire) {
        finish(app);
    }
    winutil::restore_taskbars();
}
