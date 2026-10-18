//! 捕获诊断：连续两次抓帧落盘，区分"新 duplication 首帧为黑（驱动怪癖）"
//! 与"进程被安全策略置黑"。
//! cargo run --example dump

#![cfg(windows)]

use image::RgbaImage;

fn black_ratio(img: &RgbaImage) -> f64 {
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
    black as f64 / total.max(1) as f64
}

fn grab(tag: &str) {
    match shotx_lib::capture::capture_monitors() {
        Ok(shots) => {
            for (m, f) in &shots {
                let img = RgbaImage::from_raw(f.width, f.height, f.rgba.clone());
                if let Some(img) = img {
                    let path = std::env::temp_dir().join(format!("shotx-{tag}-{}.png", m.index));
                    let _ = img.save(&path);
                    println!("{tag} {}x{} 黑像素占比 {:.1}% → {}", f.width, f.height, black_ratio(&img) * 100.0, path.display());
                }
            }
        }
        Err(e) => println!("{tag} 捕获失败: {e}"),
    }
}

fn main() {
    grab("first");
    std::thread::sleep(std::time::Duration::from_millis(700));
    grab("second");
    // 第三次：桌面此刻若有更新（鼠标移动等）应更接近真实内容
    grab("third");
}
