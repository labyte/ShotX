//! 屏幕捕获，三级后端（按实测环境能力排序）：
//! 1. **WGC（Windows.Graphics.Capture）**：DWM 合成提交路径。本机实测
//!    IPGuard 类终端软件会置黑 DXGI duplication 帧，而 WGC 不受影响，
//!    且与录屏软件不互斥（不占 duplication 名额）。
//! 2. **DXGI Desktop Duplication**：显卡合成层取帧，不经 GDI；当 WGC
//!    不可用（旧系统/异常）时使用。
//! 3. **GDI BitBlt**：远程桌面等极端环境的最后兜底。
//!
//! 诚实边界：驱动级屏幕过滤、DRM/HDCP 保护内容、设置了
//! WDA_EXCLUDEFROMCAPTURE 的窗口，在任何用户态 API 下都是黑块，非本工具
//! 规避目标。所有后端输出统一为 RGBA8 物理像素。
//!
//! 输出的像素坐标均为物理像素（进程已声明 Per-Monitor-V2 DPI 感知）。

use serde::Serialize;

#[cfg(windows)]
pub mod dxgi;
pub mod gdi;
#[cfg(windows)]
pub mod wgc;

/// 显示器信息（GDI EnumDisplayMonitors 枚举，物理像素坐标，x/y 可为负）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub index: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
}

/// 捕获帧：RGBA8888、行主序、自上而下
pub struct RawFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 枚举所有显示器
pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>, String> {
    #[cfg(windows)]
    {
        gdi::enumerate_monitors_windows()
    }
    #[cfg(not(windows))]
    {
        let _ = "当前平台暂未实现捕获";
        Err("unsupported platform".into())
    }
}

/// 捕获所有显示器，返回 (显示器, 帧)。
/// 后端优先级：WGC（DWM 提交路径，不受 duplication 置黑影响）
/// → DXGI Desktop Duplication → GDI 兜底。
pub fn capture_monitors() -> Result<Vec<(MonitorInfo, RawFrame)>, String> {
    let monitors = enumerate_monitors()?;
    if monitors.is_empty() {
        return Err("未找到可用显示器".into());
    }

    #[cfg(windows)]
    {
        match wgc::capture_all(&monitors) {
            Ok(shots) if !shots.is_empty() => {
                check_black(&shots);
                return Ok(shots);
            }
            Ok(_) => {}
            Err(e) => crate::diag::log(&format!("WGC 捕获失败，降级 DXGI: {e}")),
        }
        match dxgi::capture_all(&monitors) {
            Ok(shots) if !shots.is_empty() => {
                check_black(&shots);
                return Ok(shots);
            }
            Ok(_) => {}
            Err(e) => crate::diag::log(&format!("DXGI 捕获失败，降级 GDI: {e}")),
        }
        let shots = gdi::capture_all_windows(&monitors)?;
        check_black(&shots);
        Ok(shots)
    }
    #[cfg(not(windows))]
    {
        let _ = &monitors;
        Err("当前平台暂未实现屏幕捕获".into())
    }
}

/// BGRA → RGBA 原地转换（DXGI/GDI 取到的都是 B8G8R8A8）
pub fn bgra_to_rgba(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

/// 抽样检测捕获结果是否全黑（终端安全软件置黑/捕获管道故障的关键信号），
/// 命中则写诊断日志，便于现场定位。
fn check_black(shots: &[(MonitorInfo, RawFrame)]) {
    for (m, f) in shots {
        let px = f.width as usize * f.height as usize;
        if px == 0 || f.rgba.len() < 4 {
            continue;
        }
        let stride = (px / 2000).max(1) * 4;
        let any_nonblack = f
            .rgba
            .chunks_exact(stride)
            .any(|c| c[0] | c[1] | c[2] != 0);
        if !any_nonblack {
            crate::diag::log(&format!(
                "显示器 {}({}x{}) 捕获结果为全黑帧——可能被终端安全策略置黑，或屏幕确实处于全黑",
                m.name, m.width, m.height
            ));
        }
    }
}
