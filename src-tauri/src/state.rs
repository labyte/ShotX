use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::{Mutex, RwLock};

use crate::history::HistoryStore;
use crate::settings::Settings;

/// 单个显示器的原始捕获帧（RGBA8888、物理像素）
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 贴图窗口的状态记录
pub struct PinEntry {
    /// 合成后的 PNG 字节
    pub png: Vec<u8>,
    /// 原始图像物理尺寸（100% 时的窗口尺寸）
    pub base_w: u32,
    pub base_h: u32,
    /// 当前缩放倍率
    pub scale: f64,
}

pub struct AppState {
    pub settings: RwLock<Settings>,
    /// 当前捕获会话：monitor_index -> 帧
    pub frames: Mutex<HashMap<u32, CapturedFrame>>,
    /// 最近一次捕获的显示器几何（贴图定位、历史贴图用）
    pub monitors: Mutex<Vec<crate::capture::MonitorInfo>>,
    /// 捕获会话进行中标记（防重入）
    pub capture_active: AtomicBool,
    /// 捕获会话序号，用于丢弃迟到的旧帧回执
    pub capture_seq: AtomicU64,
    /// 已完成显示动作的会话序号
    pub shown_seq: AtomicU64,
    /// 本轮需要回执就绪的 overlay 数量与已就绪集合
    pub overlay_expected: AtomicU32,
    pub overlay_acks: Mutex<HashSet<(u64, u32)>>,
    /// 贴图注册表：label -> 条目
    pub pins: Mutex<HashMap<String, PinEntry>>,
    pub pin_counter: AtomicU32,
    /// 贴图整体显隐状态（F3）
    pub pins_visible: AtomicBool,
    /// 截图期间被临时隐藏的贴图 label
    pub pins_hidden_by_capture: Mutex<Vec<String>>,
    pub history: Mutex<Option<HistoryStore>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            settings: RwLock::new(Settings::default()),
            frames: Mutex::new(HashMap::new()),
            monitors: Mutex::new(Vec::new()),
            capture_active: AtomicBool::new(false),
            capture_seq: AtomicU64::new(0),
            shown_seq: AtomicU64::new(0),
            overlay_expected: AtomicU32::new(0),
            overlay_acks: Mutex::new(HashSet::new()),
            pins: Mutex::new(HashMap::new()),
            pin_counter: AtomicU32::new(0),
            pins_visible: AtomicBool::new(true),
            pins_hidden_by_capture: Mutex::new(Vec::new()),
            history: Mutex::new(None),
        }
    }
}
