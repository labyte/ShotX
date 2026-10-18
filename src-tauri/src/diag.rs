//! 诊断日志：追加写入 %TEMP%\shotx-diag.log。
//! release 构建无控制台，用户现场出问题时靠这个文件取证。

use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn log(msg: &str) {
    let path = std::env::temp_dir().join("shotx-diag.log");
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{ts}] {msg}");
    }
    eprintln!("[shotx] {msg}");
}
