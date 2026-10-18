//! WGC（Windows.Graphics.Capture）捕获后端 —— 主选。
//!
//! 原理：从 DWM 合成器拿每个捕获项的提交帧（GraphicsCaptureItem per monitor），
//! 与 DXGI Desktop Duplication 是不同管道。实测：部分终端安全软件（IPGuard 类）
//! 会置黑 duplication 帧，而 WGC 不受影响；且 WGC 不占用 duplication 名额，
//! 与 OBS 等录屏软件天然并存。
//!
//! 用法：每显示器起一个一次性捕获线程，拿到第一帧即停（start 内部自建
//! WinRT 初始化与 DispatcherQueue，线程隔离）。

use std::sync::mpsc;
use std::time::Duration;

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor as WgcMonitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use super::{MonitorInfo, RawFrame};

type Tx = mpsc::Sender<Result<(u32, RawFrame), String>>;

struct OneShot {
    tx: Option<Tx>,
    index: u32,
    done: bool,
}

impl GraphicsCaptureApiHandler for OneShot {
    type Flags = (u32, Tx);
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { tx: Some(ctx.flags.1), index: ctx.flags.0, done: false })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        let mut scratch = Vec::new();
        let result = (|| -> Result<RawFrame, Self::Error> {
            let buf = frame.buffer()?;
            let (w, h) = (buf.width(), buf.height());
            let rgba = buf.as_nopadding_buffer(&mut scratch).to_vec();
            Ok(RawFrame { width: w, height: h, rgba })
        })();
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(result.map(|f| (self.index, f)).map_err(|e| e.to_string()));
        }
        capture_control.stop();
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Err("捕获目标已关闭".into()));
        }
        Ok(())
    }
}

/// 用 WGC 捕获所有显示器（RGBA 直出，无需 BGRA 转换）
pub fn capture_all(monitors: &[MonitorInfo]) -> Result<Vec<(MonitorInfo, RawFrame)>, String> {
    let wgc_monitors = WgcMonitor::enumerate().map_err(|e| format!("WGC 枚举失败: {e}"))?;
    let (tx, rx) = mpsc::channel();
    let mut spawned = 0usize;

    for m in monitors {
        // 按设备名匹配（\\.\DISPLAYn），失败退回枚举顺序
        let wm = wgc_monitors
            .iter()
            .find(|w| w.device_name().map(|n| n == m.name).unwrap_or(false))
            .or_else(|| wgc_monitors.get(m.index as usize))
            .ok_or_else(|| format!("显示器 {} 无 WGC 对应项", m.name))?;
        let tx = tx.clone();
        let index = m.index;
        let monitor = *wm;
        std::thread::spawn(move || {
            let settings = Settings::new(
                monitor,
                CursorCaptureSettings::WithoutCursor,
                DrawBorderSettings::WithoutBorder,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                ColorFormat::Rgba8,
                (index, tx),
            );
            // start 失败时 flags（含 tx）已被消耗，无法回报；
            // 外层 recv_timeout 会兜住并整体降级 DXGI/GDI
            if let Err(e) = OneShot::start(settings) {
                crate::diag::log(&format!("WGC start 失败: {e:?}"));
            }
        });
        spawned += 1;
    }
    drop(tx);

    let mut out = Vec::with_capacity(spawned);
    for _ in 0..spawned {
        match rx.recv_timeout(Duration::from_millis(3000)) {
            Ok(Ok((idx, frame))) => {
                let m = monitors.iter().find(|m| m.index == idx).ok_or("索引失配")?;
                out.push((m.clone(), frame));
            }
            Ok(Err(e)) => return Err(format!("WGC 捕获错误: {e}")),
            Err(_) => return Err("WGC 等待帧超时".into()),
        }
    }
    if out.len() != monitors.len() {
        return Err("WGC 帧数与显示器数不符".into());
    }
    Ok(out)
}
