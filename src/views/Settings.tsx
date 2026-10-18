import React, { useEffect, useRef, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { invoke, listen } from "../lib/ipc";
import type { AppSettings, HistoryItem } from "../lib/ipc";

const KEY_ALIAS: Record<string, string> = {
  " ": "Space",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  ArrowUp: "Up",
  ArrowDown: "Down",
  Escape: "Esc",
  "+": "Plus",
  "-": "Minus",
};

/** 由键盘事件构造快捷键字符串，如 Ctrl+Shift+A / F1 */
function comboFromEvent(e: React.KeyboardEvent): string | null {
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return null;
  let key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
  key = KEY_ALIAS[key] ?? key;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  // 无修饰键时仅允许 F1-F12 / Esc 等独立键
  if (mods.length === 0 && !/^F\d{1,2}$/.test(key) && key !== "PrintScreen") {
    return null;
  }
  return [...mods, key].join("+");
}

function HotkeyInput({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
}) {
  const [capturing, setCapturing] = useState(false);
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      <div className="hotkey-row">
        <input
          className={`hotkey ${capturing ? "capturing" : ""}`}
          readOnly
          value={capturing ? "按下组合键…" : value || "未设置"}
          onFocus={() => setCapturing(true)}
          onBlur={() => setCapturing(false)}
          onKeyDown={(e) => {
            e.preventDefault();
            if (!capturing) return;
            if (e.key === "Backspace") {
              onChange("");
              return;
            }
            const combo = comboFromEvent(e);
            if (combo) {
              onChange(combo);
              setCapturing(false);
            }
          }}
        />
        <button className="btn mini" onClick={() => onChange("")} title="清除快捷键">
          清除
        </button>
      </div>
    </label>
  );
}

export default function Settings() {
  const [tab, setTab] = useState<"general" | "history">("general");
  const [cfg, setCfg] = useState<AppSettings | null>(null);
  const [saved, setSaved] = useState(false);
  const [items, setItems] = useState<HistoryItem[]>([]);
  const thumbs = useRef<Map<string, string>>(new Map());

  const reloadSettings = () =>
    invoke<AppSettings>("get_settings").then((s) => {
      setCfg(s);
      setSaved(false);
    });

  const reloadHistory = () => invoke<HistoryItem[]>("history_list").then(setItems);

  useEffect(() => {
    reloadSettings();
    reloadHistory();
    const un = listen("shotx://settings-refresh", () => {
      reloadSettings();
      reloadHistory();
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // 探测模式：自动保存一次，验证 update_settings 全链路（见 SHOTX_DEBUG_PROBE）
  const probedRef = useRef(false);
  useEffect(() => {
    const probe = new URLSearchParams(window.location.search).get("probe") === "1";
    if (probe && cfg && !probedRef.current) {
      probedRef.current = true;
      saveCfg(cfg);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cfg]);

  // 懒加载缩略图
  useEffect(() => {
    for (const it of items) {
      if (thumbs.current.has(it.id)) continue;
      invoke<ArrayBuffer>("history_thumb", { id: it.id }).then((buf) => {
        if (buf && buf.byteLength > 0) {
          thumbs.current.set(
            it.id,
            URL.createObjectURL(new Blob([buf], { type: "image/png" })),
          );
          setItems((old) => [...old]); // 触发重渲染
        }
      });
    }
  }, [items]);

  async function saveCfg(next: AppSettings) {
    try {
      const applied = await invoke<AppSettings>("update_settings", {
        newSettings: next,
      });
      setCfg(applied);
      invoke("refresh_tray_tooltip");
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (e) {
      alert(`保存失败: ${e}`);
    }
  }

  async function pinFromHistory(it: HistoryItem) {
    await invoke("history_pin", { id: it.id });
  }

  async function copyFromHistory(it: HistoryItem) {
    // 历史文件 → 原始字节 → copy_png
    const buf = await invoke<ArrayBuffer>("history_file", { id: it.id });
    if (buf) await invoke("copy_png", new Uint8Array(buf), { headers: {} });
  }

  const set = <K extends keyof AppSettings>(k: K, v: AppSettings[K]) =>
    setCfg((c) => (c ? { ...c, [k]: v } : c));

  return (
    <div className="settings-root">
      <header className="settings-head">
        <div className="tabs">
          <button className={tab === "general" ? "tab active" : "tab"} onClick={() => setTab("general")}>
            基本设置
          </button>
          <button className={tab === "history" ? "tab active" : "tab"} onClick={() => setTab("history")}>
            历史截图
          </button>
        </div>
        <button
          className="btn"
          onClick={() => {
            // 关闭窗口 = 隐藏复用
            invoke("hide_self");
          }}
        >
          收起
        </button>
      </header>

      {!cfg ? (
        <div className="pane empty">加载中…</div>
      ) : tab === "general" ? (
        <div className="pane">
          <HotkeyInput
            label="截图快捷键"
            value={cfg.captureHotkey}
            onChange={(v) => set("captureHotkey", v)}
          />
          <HotkeyInput
            label="贴图显隐快捷键"
            value={cfg.pinsHotkey}
            onChange={(v) => set("pinsHotkey", v)}
          />
          <label className="field check">
            <input
              type="checkbox"
              checked={cfg.autostart}
              onChange={(e) => set("autostart", e.target.checked)}
            />
            <span>开机自动启动 ShotX（默认驻留托盘）</span>
          </label>
          <label className="field check">
            <input
              type="checkbox"
              checked={cfg.hideTaskbar}
              onChange={(e) => set("hideTaskbar", e.target.checked)}
            />
            <span>截图时临时隐藏任务栏（可完整框选整个屏幕）</span>
          </label>
          <label className="field check">
            <input
              type="checkbox"
              checked={cfg.hidePinsOnCapture}
              onChange={(e) => set("hidePinsOnCapture", e.target.checked)}
            />
            <span>截图时隐藏我的贴图（默认贴图会随桌面一起入镜）</span>
          </label>
          <label className="field">
            <span className="field-label">历史保留数量</span>
            <input
              type="number"
              min={0}
              max={500}
              value={cfg.historyLimit}
              onChange={(e) => set("historyLimit", Math.max(0, Number(e.target.value) || 0))}
            />
          </label>
          <div className="row-end">
            {saved && <span className="saved-tip">已保存 ✓</span>}
            <button className="btn primary" onClick={() => saveCfg(cfg)}>
              保存设置
            </button>
          </div>
          <p className="note">
            ShotX 常驻系统托盘，无主窗口。左键单击托盘图标或按截图快捷键即可截图。
            截屏优先走 WGC（Windows.Graphics.Capture，DWM 合成提交路径），
            降级顺序 WGC → DXGI Desktop Duplication → GDI，均不经应用层钩子，
            与挂钩屏幕截取的终端类软件兼容共存（驱动级过滤/受 DRM 保护内容除外）。
          </p>
        </div>
      ) : (
        <div className="pane history">
          {items.length === 0 && <div className="empty">暂无历史记录</div>}
          <div className="hist-grid">
            {items.map((it) => (
              <div key={it.id} className="hist-card" onDoubleClick={() => pinFromHistory(it)}>
                {thumbs.current.has(it.id) ? (
                  <img src={thumbs.current.get(it.id)} alt="" />
                ) : (
                  <div className="hist-thumb-loading" />
                )}
                <div className="hist-meta">
                  {it.width}×{it.height}
                </div>
                <div className="hist-ops">
                  <button className="mini" title="贴图" onClick={() => pinFromHistory(it)}>
                    贴
                  </button>
                  <button className="mini" title="复制" onClick={() => copyFromHistory(it)}>
                    复
                  </button>
                  <button
                    className="mini"
                    title="在文件夹中显示"
                    onClick={() => revealItemInDir(it.file)}
                  >
                    目
                  </button>
                  <button
                    className="mini danger"
                    title="删除"
                    onClick={async () => {
                      await invoke("history_delete", { id: it.id });
                      reloadHistory();
                    }}
                  >
                    删
                  </button>
                </div>
              </div>
            ))}
          </div>
          {items.length > 0 && (
            <div className="row-end">
              <button
                className="btn danger"
                onClick={async () => {
                  if (confirm("确定清空全部历史截图吗？")) {
                    await invoke("history_clear");
                    reloadHistory();
                  }
                }}
              >
                清空历史
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
