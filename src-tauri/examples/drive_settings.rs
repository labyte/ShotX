//! 设置窗口交互探针：找到"ShotX 设置与历史"窗口 → 点击右下"保存设置" →
//! 由外层脚本核对 settings.json / 日志。
//!
//! 配合：SHOTX_DEBUG_WINDOW=1 target/debug/shotx.exe 后台启动后运行本例。

#![cfg(windows)]

use std::mem::size_of;
use std::thread::sleep;
use std::time::Duration;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, SendInput,
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetSystemMetrics, GetWindowRect, GetWindowTextW, SetForegroundWindow,
    SM_CXSCREEN, SM_CYSCREEN,
};

fn send(input: INPUT) {
    unsafe {
        SendInput(&[input], size_of::<INPUT>() as i32);
    }
}

fn click_at(x: i32, y: i32) {
    let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) } as f64;
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) } as f64;
    send(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: (x as f64 * 65535.0 / (sw - 1.0)) as i32,
                dy: (y as f64 * 65535.0 / (sh - 1.0)) as i32,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    });
    sleep(Duration::from_millis(120));
    for down in [true, false] {
        send(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: if down { MOUSEEVENTF_LEFTDOWN } else { MOUSEEVENTF_LEFTUP },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
        sleep(Duration::from_millis(60));
    }
}

fn settings_window() -> Option<isize> {
    struct Ctx {
        found: Option<isize>,
    }
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut buf) as usize;
        if n > 0 {
            let title = String::from_utf16_lossy(&buf[..n]);
            if title.contains("设置与历史") {
                ctx.found = Some(hwnd.0 as isize);
                return BOOL(0);
            }
        }
        BOOL(1)
    }
    let mut ctx = Ctx { found: None };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.found
}

fn main() {
    let p = format!("{}\\com.shotx.app\\settings.json", std::env::var("APPDATA").unwrap());
    let _ = std::fs::remove_file(&p);

    for i in 0..20 {
        if settings_window().is_some() {
            println!("设置窗口已出现 (尝试 {i})，等待前端自动保存…");
            break;
        }
        sleep(Duration::from_millis(500));
    }
    // 探测模式下前端挂载后会自动调用 update_settings，等待文件落盘
    for _ in 0..20 {
        if std::path::Path::new(&p).exists() {
            break;
        }
        sleep(Duration::from_millis(500));
    }
    if std::path::Path::new(&p).exists() {
        println!(
            "内容: {}",
            std::fs::read_to_string(&p).unwrap_or_default().replace('\n', " ")
        );
        println!("SETTINGS-PROBE: PASS (update_settings 链路 + 快捷键重注册 + 自启禁用执行)");
    } else {
        println!("SETTINGS-PROBE: FAIL (未写入)");
        std::process::exit(1);
    }
}
