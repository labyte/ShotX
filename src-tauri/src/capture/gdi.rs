//! GDI 显示枚举与捕获。
//! 作为 DXGI Desktop Duplication 不可用时（RDP、老驱动等）的兜底路径。

use super::{MonitorInfo, RawFrame};

#[cfg(not(windows))]
pub fn enumerate_monitors_windows() -> Result<Vec<MonitorInfo>, String> {
    Err("当前平台暂未实现显示器枚举".into())
}

#[cfg(not(windows))]
pub fn capture_all_windows(_monitors: &[MonitorInfo]) -> Result<Vec<(MonitorInfo, RawFrame)>, String> {
    Err("当前平台暂未实现屏幕捕获".into())
}

/// MONITORINFOF_PRIMARY（WindowsAndMessaging 中定义为 u32 常量 1）
const MONITORINFOF_PRIMARY: u32 = 1;

#[cfg(windows)]
mod imp {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
        EnumDisplayMonitors, GetDIBits, GetDC, GetMonitorInfoW, ReleaseDC, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFOEXW,
        SRCCOPY,
    };
    use windows::Win32::Foundation::HWND;

    use super::{MonitorInfo, RawFrame, MONITORINFOF_PRIMARY};
    use crate::capture::bgra_to_rgba;

    fn utf16_z(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    struct RawMon {
        rc: RECT,
        device: [u16; 32],
        flags: u32,
    }

    unsafe extern "system" fn enum_cb(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<RawMon>);
        let mut mi = MONITORINFOEXW::default();
        // cbSize 必须是 EXW 总大小，Windows 才会顺带填充 szDevice
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmon, std::ptr::from_mut(&mut mi.monitorInfo)).as_bool() {
            out.push(RawMon {
                rc: mi.monitorInfo.rcMonitor,
                device: mi.szDevice,
                flags: mi.monitorInfo.dwFlags,
            });
        }
        BOOL(1)
    }

    /// 枚举显示器（物理像素、GDI 顺序、含主显标记）
    pub fn enumerate_monitors_windows() -> Result<Vec<MonitorInfo>, String> {
        let mut raw: Vec<RawMon> = Vec::new();
        unsafe {
            let ok = EnumDisplayMonitors(
                None,
                None,
                Some(enum_cb),
                LPARAM(&mut raw as *mut _ as isize),
            );
            if !ok.as_bool() {
                return Err("EnumDisplayMonitors 失败".into());
            }
        }
        Ok(raw
            .into_iter()
            .enumerate()
            .map(|(i, m)| MonitorInfo {
                index: i as u32,
                name: utf16_z(&m.device),
                x: m.rc.left,
                y: m.rc.top,
                width: (m.rc.right - m.rc.left).max(0) as u32,
                height: (m.rc.bottom - m.rc.top).max(0) as u32,
                is_primary: m.flags & MONITORINFOF_PRIMARY != 0,
            })
            .collect())
    }

    /// 用 GetDC(NULL)+BitBlt+GetDIBits 截取指定显示器区域，输出 RGBA
    pub fn capture_one_windows(m: &MonitorInfo) -> Result<RawFrame, String> {
        let (w, h) = (m.width as i32, m.height as i32);
        if w <= 0 || h <= 0 {
            return Err("显示器尺寸非法".into());
        }
        unsafe {
            let screen = GetDC(Some(HWND::default()));
            let mem = CreateCompatibleDC(Some(screen));
            let bm: HBITMAP = CreateCompatibleBitmap(screen, w, h);
            let old: HGDIOBJ = SelectObject(mem, bm.into());

            let _ = BitBlt(mem, 0, 0, w, h, Some(screen), m.x, m.y, SRCCOPY);

            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // 负值 = 自上而下
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
            let got = GetDIBits(
                mem,
                bm,
                0,
                h as u32,
                Some(buf.as_mut_ptr() as *mut std::ffi::c_void),
                &mut bmi,
                DIB_RGB_COLORS,
            );

            SelectObject(mem, old);
            let _ = DeleteObject(bm.into());
            let _ = DeleteDC(mem);
            ReleaseDC(Some(HWND::default()), screen);

            if got <= 0 {
                return Err("GetDIBits 失败".into());
            }
            bgra_to_rgba(&mut buf);
            Ok(RawFrame { width: w as u32, height: h as u32, rgba: buf })
        }
    }

    pub fn capture_all_windows(monitors: &[MonitorInfo]) -> Result<Vec<(MonitorInfo, RawFrame)>, String> {
        let mut out = Vec::with_capacity(monitors.len());
        for m in monitors {
            out.push((m.clone(), capture_one_windows(m)?));
        }
        Ok(out)
    }
}

#[cfg(windows)]
pub use imp::{capture_all_windows, enumerate_monitors_windows};
