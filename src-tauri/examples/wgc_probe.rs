//! WGC（Windows.Graphics.Capture）后端探针：验证本机 WGC 路径能否拿到正常帧。
//! 与 DXGI duplication 黑帧问题对照。cargo run --example wgc_probe

#![cfg(windows)]

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::ImageFormat;
use windows_capture::frame::Frame;
use image::GenericImageView;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

struct Probe {
    done: bool,
}

impl GraphicsCaptureApiHandler for Probe {
    type Flags = ();
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(_ctx: Context<()>) -> Result<Self, Self::Error> {
        Ok(Self { done: false })
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
        let path = std::env::temp_dir().join("wgc-probe.png");
        frame.save_as_image(&path, ImageFormat::Png)?;
        let img = image::open(&path)?;
        let (w, h) = img.dimensions();
        let mut black = 0u64;
        let mut total = 0u64;
        for y in (0..h).step_by(7) {
            for x in (0..w).step_by(7) {
                let p = img.get_pixel(x, y);
                total += 1;
                if p[0] as u32 + p[1] as u32 + p[2] as u32 <= 24 {
                    black += 1;
                }
            }
        }
        println!(
            "WGC 帧 {}x{} 黑像素占比 {:.1}% → {}",
            w,
            h,
            black as f64 * 100.0 / total.max(1) as f64,
            path.display()
        );
        capture_control.stop();
        Ok(())
    }
}

fn main() {
    let monitor = Monitor::primary().expect("无主显示器");
    let item: GraphicsCaptureItemType = monitor
        .try_into()
        .expect("Monitor 转 capture item 失败");
    let settings = Settings::new(
        item,
        CursorCaptureSettings::Default,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        (),
    );
    Probe::start(settings).expect("WGC 启动失败");
}
