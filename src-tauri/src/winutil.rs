//! Windows 平台杂项：DPI 感知、任务栏临时隐藏、分层窗口透明度。

use std::sync::Mutex;
use std::sync::OnceLock;

/// 记录截图期间被隐藏的任务栏窗口（HWND 以 isize 存储，跨线程安全）
static HIDDEN_TRAYS: OnceLock<Mutex<Vec<isize>>> = OnceLock::new();

fn trays() -> &'static Mutex<Vec<isize>> {
    HIDDEN_TRAYS.get_or_init(|| Mutex::new(Vec::new()))
}

#[cfg(windows)]
mod imp {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM};
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, SetLayeredWindowAttributes, ShowWindow, LWA_ALPHA, SW_HIDE,
        SW_SHOW,
    };

    use super::trays;

    /// 声明 Per-Monitor-V2 DPI 感知（若 tao 已先行设置，此调用失败但无副作用）
    pub fn set_dpi_awareness() {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    unsafe extern "system" fn find_tray(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<HWND>);
        let mut cls = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut cls) as usize;
        if n > 0 {
            let name = String::from_utf16_lossy(&cls[..n]);
            if name == "Shell_TrayWnd" || name == "Shell_SecondaryTrayWnd" {
                out.push(hwnd);
            }
        }
        BOOL(1)
    }

    /// 隐藏所有任务栏（覆盖层需要盖住整个屏幕，包括任务栏区域）
    pub fn hide_taskbars() {
        unsafe {
            let mut found: Vec<HWND> = Vec::new();
            let _ = EnumWindows(Some(find_tray), LPARAM(&mut found as *mut _ as isize));
            for h in &found {
                let _ = ShowWindow(*h, SW_HIDE);
            }
            let mut guard = trays().lock().unwrap();
            if guard.is_empty() {
                *guard = found.iter().map(|h| h.0 as isize).collect();
            }
        }
    }

    /// 恢复之前隐藏的任务栏
    pub fn restore_taskbars() {
        // HWND 内部字段类型随 windows 版本变化（指针或 isize），用 as _ 兼容
        let list: Vec<HWND> = trays()
            .lock()
            .unwrap()
            .drain(..)
            .map(|v| HWND(v as _))
            .collect();
        unsafe {
            for h in list {
                let _ = ShowWindow(h, SW_SHOW);
            }
        }
    }

    /// 用 SetLayeredWindowAttributes 设置整窗 alpha（贴图透明度）
    pub fn set_window_alpha(hwnd: isize, alpha: f64) -> bool {
        let a = (alpha.clamp(0.05, 1.0) * 255.0).round() as u8;
        unsafe {
            SetLayeredWindowAttributes(HWND(hwnd as _), COLORREF(0), a, LWA_ALPHA)
                .is_ok()
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn set_dpi_awareness() {}
    pub fn hide_taskbars() {}
    pub fn restore_taskbars() {}
    pub fn set_window_alpha(_hwnd: isize, _alpha: f64) -> bool {
        false
    }
}

pub use imp::{hide_taskbars, restore_taskbars, set_dpi_awareness, set_window_alpha};
