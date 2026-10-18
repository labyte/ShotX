//! 端到端冒烟测试（开发辅助）：
//! F1 → 覆盖层出现 → 鼠标框选 → 双击完成复制 → 校验历史文件与剪贴板位图 →
//! 覆盖层应已隐藏。结束时无条件恢复任务栏显示。
//!
//! 用法：先 `npm run tauri dev`，再 `cargo run --example drive`。
//! 假设主显示器分辨率 ≥ 1024x600 且原点 (0,0)。

#![cfg(windows)]

use std::mem::size_of;
use std::thread::sleep;
use std::time::Duration;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::DataExchange::{
    CloseClipboard, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::Ole::CF_BITMAP;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_WHEEL, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_KEYUP, MOUSEINPUT, VK_ESCAPE, VK_F1, VK_P, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetSystemMetrics, GetWindowRect, GetWindowTextW, IsWindowVisible,
    SetForegroundWindow, ShowWindow, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW,
};

fn send(input: INPUT) {
    unsafe {
        SendInput(&[input], size_of::<INPUT>() as i32);
    }
}

fn press(vk: VIRTUAL_KEY) {
    send(INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: Default::default(),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    });
    sleep(Duration::from_millis(40));
    send(INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    });
}

fn screen() -> (f64, f64) {
    unsafe {
        (
            GetSystemMetrics(SM_CXSCREEN) as f64,
            GetSystemMetrics(SM_CYSCREEN) as f64,
        )
    }
}

fn move_mouse(x: i32, y: i32) {
    let (sw, sh) = screen();
    let dx = ((x as f64) * 65535.0 / (sw - 1.0)) as i32;
    let dy = ((y as f64) * 65535.0 / (sh - 1.0)) as i32;
    send(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    });
}

fn left(down: bool) {
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
}

fn drag(from: (i32, i32), to: (i32, i32)) {
    move_mouse(from.0, from.1);
    sleep(Duration::from_millis(80));
    left(true);
    for i in 1..=10 {
        let x = from.0 + (to.0 - from.0) * i / 10;
        let y = from.1 + (to.1 - from.1) * i / 10;
        move_mouse(x, y);
        sleep(Duration::from_millis(25));
    }
    left(false);
    sleep(Duration::from_millis(150));
}

fn double_click(p: (i32, i32)) {
    move_mouse(p.0, p.1);
    sleep(Duration::from_millis(60));
    for _ in 0..2 {
        left(true);
        sleep(Duration::from_millis(30));
        left(false);
        sleep(Duration::from_millis(60));
    }
}

fn shotx_windows() -> Vec<(String, bool, isize)> {
    struct Ctx {
        out: Vec<(String, bool, isize)>,
    }
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut buf) as usize;
        if n > 0 {
            let title = String::from_utf16_lossy(&buf[..n]);
            if title.starts_with("ShotX") {
                ctx.out.push((title, IsWindowVisible(hwnd).as_bool(), hwnd.0 as isize));
            }
        }
        BOOL(1)
    }
    let mut ctx = Ctx { out: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.out
}

fn unhide_taskbars() {
    unsafe extern "system" fn cb(hwnd: HWND, _l: LPARAM) -> BOOL {
        let mut cls = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut cls) as usize;
        if n > 0 {
            let name = String::from_utf16_lossy(&cls[..n]);
            if name == "Shell_TrayWnd" || name == "Shell_SecondaryTrayWnd" {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }
        BOOL(1)
    }
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(0));
    }
}

fn wheel(amount: i32) {
    send(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: amount as u32,
                dwFlags: MOUSEEVENTF_WHEEL,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    });
}

fn pin_window() -> Option<isize> {
    shotx_windows()
        .into_iter()
        .find(|(t, _, _)| t.contains("贴图"))
        .map(|(_, _, h)| h)
}

fn win_rect(hwnd: isize) -> (i32, i32) {
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(HWND(hwnd as _), &mut r);
        (r.right - r.left, r.bottom - r.top)
    }
}

fn history_latest() -> Option<(String, u64)> {
    let dir = format!("{}\\com.shotx.app\\history", std::env::var("APPDATA").ok()?);
    let mut best: Option<(std::time::SystemTime, String, u64)> = None;
    for entry in std::fs::read_dir(&dir).ok()? {
        let p = entry.ok()?.path();
        if p.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let meta = std::fs::metadata(&p).ok()?;
        let mtime = meta.modified().ok()?;
        if best.as_ref().map_or(true, |(t, _, _)| mtime > *t) {
            best = Some((mtime, p.display().to_string(), meta.len()));
        }
    }
    let (_, name, size) = best?;
    Some((name, size))
}

fn clipboard_has_bitmap() -> bool {
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let ok = IsClipboardFormatAvailable(CF_BITMAP.0 as u32).is_ok();
        let _ = CloseClipboard();
        ok
    }
}

fn main() {
    let mut pass = true;

    // 预热轮：创建窗口 + 等页面冷加载完成（vite dev 模块图首次加载要几秒），再 Esc 结束
    println!("== 0. 预热（F1→等页面就绪→Esc）==");
    press(VK_F1);
    sleep(Duration::from_millis(8000));
    press(VK_ESCAPE);
    sleep(Duration::from_millis(1500));
    let wins0 = shotx_windows();
    let warm_hidden = !wins0.iter().any(|(t, v, _)| t.contains("截图") && *v);
    println!("预热后覆盖层已隐藏（证明热键/Esc 通路）: {warm_hidden}");

    println!("== 1. F1 触发截图 ==");
    press(VK_F1);
    sleep(Duration::from_millis(2500));
    let wins = shotx_windows();
    println!("窗口列表: {:?}", wins);
    let Some((_, visible, hwnd)) = wins.iter().find(|(t, v, _)| t.contains("截图") && *v) else {
        println!("覆盖层未出现: FAIL");
        unhide_taskbars();
        std::process::exit(1);
    };
    let _ = visible;
    unsafe {
        let _ = SetForegroundWindow(HWND(*hwnd as _));
    }
    sleep(Duration::from_millis(800));

    println!("== 2. 鼠标框选 (300,300)→(700,500) ==");
    drag((300, 300), (700, 500));

    println!("== 3. 双击选区中心完成复制 ==");
    double_click((500, 400));
    sleep(Duration::from_millis(2500));

    println!("== 4. 校验 ==");
    let wins2 = shotx_windows();
    let overlay_gone = !wins2.iter().any(|(t, v, _)| t.contains("截图") && *v);
    println!("覆盖层已隐藏: {overlay_gone}");
    pass &= overlay_gone;

    match history_latest() {
        Some((name, size)) => {
            println!("历史文件: {name} ({size} 字节)");
            pass &= size > 1000;
        }
        None => {
            println!("历史文件: 缺失!");
            pass = false;
        }
    }

    let clip = clipboard_has_bitmap();
    println!("剪贴板位图: {clip}");
    pass &= clip;

    println!("== 5. 贴图（Pin）闭环 ==");
    press(VK_F1);
    sleep(Duration::from_millis(2000));
    let wins3 = shotx_windows();
    if let Some((_, _, h)) = wins3.iter().find(|(t, v, _)| t.contains("截图") && *v) {
        unsafe {
            let _ = SetForegroundWindow(HWND(*h as _));
        }
        sleep(Duration::from_millis(500));
        drag((200, 200), (500, 420));
        press(VK_P);
        println!("已按 P，等待贴图窗口就绪…");
        sleep(Duration::from_millis(8000));
        match pin_window() {
            Some(ph) => {
                let visible = shotx_windows()
                    .iter()
                    .any(|(t, v, _)| t.contains("贴图") && *v);
                println!("贴图窗口可见（pin_ready 生效）: {visible}");
                pass &= visible;
                if visible {
                    let (w0, _) = win_rect(ph);
                    move_mouse(350, 300);
                    for _ in 0..3 {
                        wheel(120);
                        sleep(Duration::from_millis(200));
                    }
                    sleep(Duration::from_millis(600));
                    let (w1, _) = win_rect(ph);
                    println!("滚轮缩放宽度: {w0} → {w1}");
                    pass &= w1 > w0;
                    // 拖拽测试：贴图中心按下，向右下移动 (+150,+100)，窗口应同步位移
                    let mut r = RECT::default();
                    unsafe {
                        let _ = GetWindowRect(HWND(ph as _), &mut r);
                        let _ = SetForegroundWindow(HWND(ph as _));
                    }
                    sleep(Duration::from_millis(400));
                    let cx = (r.left + r.right) / 2;
                    let cy = (r.top + r.bottom) / 2;
                    move_mouse(cx, cy);
                    sleep(Duration::from_millis(80));
                    left(true);
                    for i in 1..=6 {
                        move_mouse(cx + i * 25, cy + i * 17);
                        sleep(Duration::from_millis(45));
                    }
                    left(false);
                    sleep(Duration::from_millis(500));
                    let mut r2 = RECT::default();
                    unsafe {
                        let _ = GetWindowRect(HWND(ph as _), &mut r2);
                    }
                    let dx = r2.left - r.left;
                    let dy = r2.top - r.top;
                    println!("拖拽位移: 期望≈(150,102) 实际({dx},{dy})");
                    pass &= (dx - 150).abs() < 40 && (dy - 102).abs() < 40;

                    // 双击贴图中心 → 关闭
                    double_click(((r2.left + r2.right) / 2, (r2.top + r2.bottom) / 2));
                    sleep(Duration::from_millis(1200));
                    let gone = pin_window().is_none();
                    println!("双击关闭贴图: {gone}");
                    pass &= gone;
                }
            }
            None => {
                println!("贴图窗口未创建: FAIL");
                pass = false;
            }
        }
    } else {
        println!("第二轮覆盖层未出现: FAIL");
        pass = false;
    }

    unhide_taskbars();
    if pass {
        println!("SMOKE-TEST: PASS");
    } else {
        println!("SMOKE-TEST: FAIL");
        std::process::exit(1);
    }
}
