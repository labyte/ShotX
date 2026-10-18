use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// 历史截图条目。file/thumb 存绝对路径，便于 opener 在文件夹中显示
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: String,
    /// Unix 毫秒时间戳
    pub created: u64,
    pub width: u32,
    pub height: u32,
    pub file: PathBuf,
    pub thumb: PathBuf,
}

pub struct HistoryStore {
    dir: PathBuf,
    items: Vec<HistoryItem>,
}

impl HistoryStore {
    /// 打开（或初始化）历史目录，加载索引并清理孤儿文件
    pub fn open(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(dir.join("thumbs"));
        let index_path = dir.join("index.json");
        let mut items: Vec<HistoryItem> = fs::read_to_string(&index_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // 丢弃索引里文件已丢失的条目
        items.retain(|it| it.file.exists());
        let mut store = Self { dir, items };
        store.clean_orphans();
        store
    }

    /// 删除没有索引记录的历史图片/缩略图（崩溃残留）
    fn clean_orphans(&mut self) {
        let known: Vec<String> = self.items.iter().map(|i| i.id.clone()).collect();
        for (sub, suffix) in [("", ".png"), ("thumbs", ".png")] {
            let d = if sub.is_empty() { self.dir.clone() } else { self.dir.join(sub) };
            if let Ok(rd) = fs::read_dir(&d) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) != Some("png") {
                        continue;
                    }
                    let stem = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    if !known.contains(&stem) {
                        let _ = fs::remove_file(&p);
                    }
                    let _ = suffix;
                }
            }
        }
    }

    /// 追加一条历史（PNG 字节），生成缩略图并按上限裁剪
    pub fn push(&mut self, png: &[u8], limit: usize) -> Result<HistoryItem, String> {
        use image::GenericImageView;
        let img = image::load_from_memory(png).map_err(|e| e.to_string())?;
        let (w, h) = img.dimensions();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis() as u64;
        let mut id = now.to_string();
        while self.items.iter().any(|i| i.id == id) {
            id = (id.parse::<u64>().unwrap() + 1).to_string();
        }

        let file = self.dir.join(format!("{id}.png"));
        let thumb = self.dir.join("thumbs").join(format!("{id}.png"));
        fs::write(&file, png).map_err(|e| e.to_string())?;
        let small = img.thumbnail(320, 320);
        let mut thumb_bytes = Vec::new();
        small
            .write_to(
                &mut std::io::Cursor::new(&mut thumb_bytes),
                image::ImageFormat::Png,
            )
            .map_err(|e| e.to_string())?;
        fs::write(&thumb, thumb_bytes).map_err(|e| e.to_string())?;

        let item = HistoryItem { id, created: now, width: w, height: h, file, thumb };
        self.items.insert(0, item.clone());
        self.trim(limit);
        self.save_index()?;
        Ok(item)
    }

    fn trim(&mut self, limit: usize) {
        while self.items.len() > limit {
            if let Some(it) = self.items.pop() {
                let _ = fs::remove_file(&it.file);
                let _ = fs::remove_file(&it.thumb);
            }
        }
    }

    pub fn list(&self) -> Vec<HistoryItem> {
        self.items.clone()
    }

    pub fn item(&self, id: &str) -> Option<HistoryItem> {
        self.items.iter().find(|i| i.id == id).cloned()
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            let it = self.items.remove(pos);
            let _ = fs::remove_file(&it.file);
            let _ = fs::remove_file(&it.thumb);
            self.save_index()?;
        }
        Ok(())
    }

    pub fn clear(&mut self) -> Result<(), String> {
        for it in self.items.drain(..) {
            let _ = fs::remove_file(&it.file);
            let _ = fs::remove_file(&it.thumb);
        }
        self.save_index()
    }

    /// 应用设置里的保留数量变化时调用
    pub fn apply_limit(&mut self, limit: usize) -> Result<(), String> {
        self.trim(limit);
        self.save_index()
    }

    fn save_index(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.items).map_err(|e| e.to_string())?;
        fs::write(self.dir.join("index.json"), json).map_err(|e| e.to_string())
    }

}
