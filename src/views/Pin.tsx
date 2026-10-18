import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { PhysicalPosition } from "@tauri-apps/api/dpi";
import { save } from "@tauri-apps/plugin-dialog";
import { invoke, listen, packRequest, timestampName } from "../lib/ipc";

/** 贴图窗口：拖动 = 原生拖拽，滚轮 = 缩放，右键 = 系统原生菜单，双击/Esc = 关闭 */
export default function Pin({ id }: { id: string }) {
  const win = getCurrentWindow();
  const imgRef = useRef<HTMLImageElement>(null);
  const [loaded, setLoaded] = useState(false);
  const [scalePct, setScalePct] = useState<number | null>(null);
  const pngRef = useRef<ArrayBuffer | null>(null);
  const scaleRef = useRef(1);
  const ontopRef = useRef(true);
  const pctTimer = useRef<number>(0);

  useEffect(() => {
    (async () => {
      try {
        const buf = await invoke<ArrayBuffer>("get_pin_payload");
        if (!buf || buf.byteLength < 8) return;
        pngRef.current = buf;
        const url = URL.createObjectURL(new Blob([buf], { type: "image/png" }));
        const img = imgRef.current!;
        img.onload = async () => {
          setLoaded(true);
          const cur = await invoke<number | null>("pin_get_scale");
          if (cur && cur !== 1) scaleRef.current = cur;
          await invoke("pin_ready");
        };
        img.src = url;
      } catch (e) {
        console.error(e);
      }
    })();
  }, [id]);

  async function doCopy() {
    if (!pngRef.current) return;
    await invoke("copy_png", new Uint8Array(pngRef.current), { headers: {} });
  }

  async function doSave() {
    if (!pngRef.current) return;
    const path = await save({
      defaultPath: timestampName(),
      filters: [{ name: "PNG 图片", extensions: ["png"] }],
    });
    if (path) {
      await invoke("write_png", packRequest({ path }, new Uint8Array(pngRef.current)), {
        headers: {},
      });
    }
  }

  async function applyOpacity(v: number) {
    try {
      await invoke("pin_set_opacity", { opacity: v });
    } catch {
      if (imgRef.current) imgRef.current.style.opacity = String(v);
    }
  }

  async function setScale(next: number) {
    const applied = await invoke<number>("pin_set_scale", { scale: next });
    scaleRef.current = applied;
    setScalePct(Math.round(applied * 100));
    window.clearTimeout(pctTimer.current);
    pctTimer.current = window.setTimeout(() => setScalePct(null), 900);
  }

  // 原生菜单动作回传
  useEffect(() => {
    const un = listen<string>("pin-menu-action", async (e) => {
      const a = e.payload;
      if (a === "copy") await doCopy();
      else if (a === "save") await doSave();
      else if (a.startsWith("op")) await applyOpacity(parseFloat(a.slice(2)));
      else if (a === "top") {
        ontopRef.current = !ontopRef.current;
        await win.setAlwaysOnTop(ontopRef.current);
      } else if (a === "actual") await setScale(1);
      else if (a === "close") await invoke("pin_close");
    });
    return () => {
      un.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 滚轮缩放（窗口中心锚定，Rust 侧完成 resize）
  useEffect(() => {
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      setScale(scaleRef.current * (e.deltaY < 0 ? 1.1 : 1 / 1.1));
    };
    window.addEventListener("wheel", onWheel, { passive: false });
    return () => window.removeEventListener("wheel", onWheel);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Esc 关闭
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") invoke("pin_close");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // 手动拖拽：startDragging 在 Windows 走 WM_NCLBUTTONDOWN 模态循环，
  // 按键松开后再触发会产生"幽灵拖拽"（拖不动/方向错乱/要拖到边缘）。
  // 改为按下记录起点、移动时 setPosition 直接挪窗口，行为确定且双击不被吞。
  const drag = useRef<{
    sx: number;
    sy: number;
    ox: number;
    oy: number;
    ready: boolean;
    active: boolean;
  } | null>(null);

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      const d = drag.current;
      if (!d?.active || !d.ready) return;
      const scale = window.devicePixelRatio || 1;
      win.setPosition(
        new PhysicalPosition(
          Math.round(d.ox + (e.screenX - d.sx) * scale),
          Math.round(d.oy + (e.screenY - d.sy) * scale),
        ),
      );
    };
    const onUp = () => {
      if (drag.current) drag.current.active = false;
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const startDrag = (e: React.MouseEvent) => {
    const d = { sx: e.screenX, sy: e.screenY, ox: 0, oy: 0, ready: false, active: true };
    drag.current = d;
    win.outerPosition().then((p) => {
      if (drag.current === d) {
        d.ox = p.x;
        d.oy = p.y;
        d.ready = true;
      }
    });
  };

  return (
    <div
      className="pin-root"
      onContextMenu={(e) => {
        e.preventDefault();
        invoke("pin_menu", { x: e.clientX, y: e.clientY, ontop: ontopRef.current });
      }}
      onMouseDown={(e) => {
        if (e.button !== 0) return;
        if (e.detail >= 2) {
          drag.current = null;
          invoke("pin_close");
          return;
        }
        startDrag(e);
      }}
    >
      <img ref={imgRef} className={`pin-img ${loaded ? "show" : ""}`} alt="" draggable={false} />
      {scalePct !== null && <div className="pin-pct">{scalePct}%</div>}
    </div>
  );
}
