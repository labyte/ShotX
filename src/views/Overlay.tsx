import React, { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import {
  invoke,
  packRequest,
  frameBitmap,
  timestampName,
} from "../lib/ipc";
import {
  COLORS,
  WIDTHS,
  Shape,
  Sel,
  Pt,
  Tool,
  buildPixelated,
  renderAnnotations,
  compositePNG,
} from "../lib/engine";

interface FrameReady {
  seq: number;
  monitor: number;
  width: number;
  height: number;
  monitorX: number;
  monitorY: number;
  isPrimary: boolean;
}

type DragMode =
  | { kind: "none" }
  | { kind: "newsel"; start: Pt }
  | { kind: "move"; start: Pt; origin: Sel }
  | { kind: "resize"; handle: number; origin: Sel }
  | { kind: "shape" };

const CURSORS = ["nwse-resize", "ns-resize", "nesw-resize", "ew-resize", "nwse-resize", "ns-resize", "nesw-resize", "ew-resize"];

/** 把手索引 → 在 sel 上的锚点（物理像素） */
function handlePos(sel: Sel, i: number): Pt {
  const cx = sel.x + sel.w / 2;
  const cy = sel.y + sel.h / 2;
  const xs = [sel.x, cx, sel.x + sel.w];
  const ys = [sel.y, cy, sel.y + sel.h];
  // 0:nw 1:n 2:ne 3:e 4:se 5:s 6:sw 7:w
  const map = [
    [0, 0], [1, 0], [2, 0], [2, 1], [2, 2], [1, 2], [0, 2], [0, 1],
  ];
  return { x: xs[map[i][0]], y: ys[map[i][1]] };
}

const TOOL_DEFS: { tool: Tool; label: string; icon: React.ReactNode }[] = [
  { tool: "rect", label: "矩形", icon: <rect x="4" y="6" width="16" height="12" /> },
  { tool: "ellipse", label: "椭圆", icon: <ellipse cx="12" cy="12" rx="8" ry="6" /> },
  { tool: "line", label: "直线", icon: <line x1="5" y1="19" x2="19" y2="5" /> },
  {
    tool: "arrow",
    label: "箭头",
    icon: (
      <g>
        <line x1="5" y1="19" x2="17" y2="7" />
        <polygon points="19,5 12,7 17,12" stroke="none" fill="currentColor" />
      </g>
    ),
  },
  { tool: "pen", label: "画笔", icon: <path d="M4,18 C8,8 14,20 20,8" fill="none" /> },
  {
    tool: "mosaic",
    label: "马赛克",
    icon: (
      <g stroke="none" fill="currentColor">
        <rect x="4" y="5" width="7" height="6" />
        <rect x="13" y="5" width="7" height="6" opacity=".55" />
        <rect x="4" y="13" width="7" height="6" opacity=".55" />
        <rect x="13" y="13" width="7" height="6" />
      </g>
    ),
  },
  {
    tool: "text",
    label: "文字",
    icon: (
      <text x="12" y="18" textAnchor="middle" fontSize="17" fontWeight="700" stroke="none" fill="currentColor">
        A
      </text>
    ),
  },
];

export default function Overlay({ monitor }: { monitor: number }) {
  const dpr = window.devicePixelRatio || 1;
  const bgRef = useRef<HTMLCanvasElement>(null);
  const annRef = useRef<HTMLCanvasElement>(null);
  const magRef = useRef<HTMLCanvasElement>(null);

  const frameRef = useRef<{ bmp: ImageBitmap; w: number; h: number; pixel: HTMLCanvasElement } | null>(null);
  const [hasFrame, setHasFrame] = useState(false);
  const [sel, setSel] = useState<Sel | null>(null);
  const [shapes, setShapes] = useState<Shape[]>([]);
  const [redoStack, setRedoStack] = useState<Shape[]>([]);
  const [tool, setTool] = useState<Tool | null>(null);
  const [color, setColor] = useState(COLORS[0]);
  const [widthIdx, setWidthIdx] = useState(1);
  const [textDraft, setTextDraft] = useState<{ at: Pt; value: string } | null>(null);
  // 拖拽中标记（state 驱动：pointerup 后必须触发重渲染才能显示工具栏，
  // 读 ref 不会触发渲染 → 工具栏"时隐时现"的经典 bug）
  const [dragging, setDragging] = useState(false);

  const selRef = useRef<Sel | null>(null);
  const dragRef = useRef<DragMode>({ kind: "none" });
  const activeRef = useRef<Shape | null>(null);
  const shapesRef = useRef<Shape[]>([]);
  const shapeId = useRef(1);
  const finishedRef = useRef(false);

  selRef.current = sel;
  shapesRef.current = shapes;

  // ---------- 渲染 ----------

  const draw = useCallback(() => {
    const bg = bgRef.current;
    const ann = annRef.current;
    const f = frameRef.current;
    if (!bg || !ann || !f) return;
    const ctx = bg.getContext("2d")!;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, bg.width, bg.height);
    ctx.drawImage(f.bmp, 0, 0);
    ctx.fillStyle = "rgba(0,0,0,0.45)";
    ctx.fillRect(0, 0, bg.width, bg.height);
    const s = selRef.current;
    if (s) {
      ctx.drawImage(f.bmp, s.x, s.y, s.w, s.h, s.x, s.y, s.w, s.h);
    }
    const actx = ann.getContext("2d")!;
    renderAnnotations(actx, shapesRef.current, activeRef.current, s ?? { x: 0, y: 0, w: f.w, h: f.h }, f.pixel, dpr);
  }, [dpr]);

  useEffect(() => {
    draw();
  }, [draw, sel, hasFrame]);

  // ---------- 取帧 ----------

  // 挂载：必须先完成事件订阅，再握手（overlay_ready 可能触发 Rust 立即补发
  // frame-ready，订阅未完成时事件会被丢弃）
  let unlistenRef: (() => void) | undefined;
  useEffect(() => {
    let disposed = false;
    (async () => {
      const un = await listen<FrameReady>("shotx://frame-ready", handler);
      if (disposed) {
        un();
        return;
      }
      unlistenRef = un;
      await invoke("overlay_ready", { monitor }).catch(() => {});
    })();
    return () => {
      disposed = true;
      unlistenRef?.();
    };

    async function handler(e: { payload: FrameReady }) {
      const p = e.payload;
      if (p.monitor !== monitor) return;
      const buf = await invoke<ArrayBuffer>("take_capture_frame", { monitor });
      const f = await frameBitmap(buf);
      if (!f) {
        // 拿不到帧：终止会话，绝不留下黑色等待层
        await invoke("overlay_finish").catch(() => {});
        return;
      }
      const bg = bgRef.current!;
      const ann = annRef.current!;
      bg.width = f.w;
      bg.height = f.h;
      ann.width = f.w;
      ann.height = f.h;
      frameRef.current = {
        bmp: f.bmp,
        w: f.w,
        h: f.h,
        pixel: buildPixelated(f.bmp, f.w, f.h, Math.max(6, Math.round(8 * dpr))),
      };
      // 窗口复用：必须重置上一轮的选区/标注/工具状态
      selRef.current = null;
      shapesRef.current = [];
      dragRef.current = { kind: "none" };
      activeRef.current = null;
      finishedRef.current = false;
      setDragging(false);
      setSel(null);
      setShapes([]);
      setRedoStack([]);
      setTool(null);
      setTextDraft(null);
      setHasFrame(true);
      // 关键：回执前同步绘出首帧，杜绝"窗口已显示但还没画"的黑底窗口期
      draw();
      await invoke("overlay_ack", { monitor });
    }
  }, [monitor, dpr]);

  // ---------- 指针交互 ----------

  const toPhys = (e: React.PointerEvent | React.MouseEvent): Pt => ({
    x: Math.round(e.clientX * dpr),
    y: Math.round(e.clientY * dpr),
  });

  const clamp = (s: Sel): Sel => {
    const f = frameRef.current;
    if (!f) return s;
    const x = Math.max(0, Math.min(s.x, f.w - 1));
    const y = Math.max(0, Math.min(s.y, f.h - 1));
    const w = Math.max(1, Math.min(s.w, f.w - x));
    const h = Math.max(1, Math.min(s.h, f.h - y));
    return { x, y, w, h };
  };

  const hitHandle = (p: Pt, s: Sel): number => {
    const tol = 10 * dpr;
    for (let i = 0; i < 8; i++) {
      const hp = handlePos(s, i);
      if (Math.abs(p.x - hp.x) <= tol && Math.abs(p.y - hp.y) <= tol) return i;
    }
    return -1;
  };

  const inside = (p: Pt, s: Sel) =>
    p.x >= s.x && p.x <= s.x + s.w && p.y >= s.y && p.y <= s.y + s.h;

  function onPointerDown(e: React.PointerEvent) {
    if (!frameRef.current || e.button !== 0) return;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    const p = toPhys(e);

    if (textDraft) {
      commitText();
      return;
    }

    const s = selRef.current;
    if (tool === "text") return; // 文字由 click 定位（见 onClickTextToolPlace）
    if (tool && s) {
      // 标注绘制
      setDragging(true);
      const shape: Shape = {
        id: shapeId.current++,
        tool,
        color,
        width: WIDTHS[widthIdx],
        points:
          tool === "rect" || tool === "ellipse" || tool === "line" || tool === "arrow"
            ? [p, { ...p }]
            : [p],
      };
      activeRef.current = shape;
      dragRef.current = { kind: "shape" };
      return;
    }

    if (!s) {
      dragRef.current = { kind: "newsel", start: p };
      setDragging(true);
      setSel({ x: p.x, y: p.y, w: 0, h: 0 });
      return;
    }
    const hIdx = hitHandle(p, s);
    if (hIdx >= 0) {
      dragRef.current = { kind: "resize", handle: hIdx, origin: s };
      setDragging(true);
      return;
    }
    if (inside(p, s)) {
      dragRef.current = { kind: "move", start: p, origin: s };
      setDragging(true);
      return;
    }
    // 重新框选
    setShapes([]);
    setRedoStack([]);
    dragRef.current = { kind: "newsel", start: p };
    setDragging(true);
    setSel({ x: p.x, y: p.y, w: 0, h: 0 });
  }

  function onPointerMove(e: React.PointerEvent) {
    if (!frameRef.current) return;
    const p = toPhys(e);
    updateMagnifier(e, p);
    const d = dragRef.current;
    switch (d.kind) {
      case "newsel": {
        const x = Math.min(d.start.x, p.x);
        const y = Math.min(d.start.y, p.y);
        const w = Math.abs(p.x - d.start.x);
        const h = Math.abs(p.y - d.start.y);
        selRef.current = clamp({ x, y, w, h });
        setSel(selRef.current);
        break;
      }
      case "move": {
        const dx = p.x - d.start.x;
        const dy = p.y - d.start.y;
        selRef.current = clamp({ ...d.origin, x: d.origin.x + dx, y: d.origin.y + dy });
        setSel(selRef.current);
        break;
      }
      case "resize": {
        const o = d.origin;
        let { x, y, w, h } = o;
        // 把手 0..7，对位调整
        if (d.handle === 0 || d.handle === 6 || d.handle === 7) {
          w = o.x + o.w - p.x;
          x = p.x;
          if (w < 1) {
            w = 1;
            x = o.x + o.w - 1;
          }
        }
        if (d.handle === 2 || d.handle === 3 || d.handle === 4) {
          w = p.x - o.x;
          x = o.x;
          if (w < 1) w = 1;
        }
        if (d.handle === 0 || d.handle === 1 || d.handle === 2) {
          h = o.y + o.h - p.y;
          y = p.y;
          if (h < 1) {
            h = 1;
            y = o.y + o.h - 1;
          }
        }
        if (d.handle === 4 || d.handle === 5 || d.handle === 6) {
          h = p.y - o.y;
          y = o.y;
          if (h < 1) h = 1;
        }
        selRef.current = clamp({ x, y, w, h });
        setSel(selRef.current);
        break;
      }
      case "shape": {
        const a = activeRef.current;
        if (!a) break;
        if (a.tool === "pen" || a.tool === "mosaic") {
          a.points.push(p);
        } else if (a.tool === "text") {
          // 文字在下钻时处理
        } else {
          a.points[1] = { ...p };
        }
        draw();
        break;
      }
    }
  }

  function onPointerUp() {
    const d = dragRef.current;
    dragRef.current = { kind: "none" };
    setDragging(false);
    hideMagnifier();
    if (d.kind === "shape") {
      const a = activeRef.current;
      activeRef.current = null;
      if (!a) return;
      if (a.tool === "rect" || a.tool === "ellipse") {
        const r = a.points[1];
        if (Math.abs(r.x - a.points[0].x) < 2 && Math.abs(r.y - a.points[0].y) < 2) {
          draw();
          return;
        }
      }
      if ((a.tool === "line" || a.tool === "arrow") && Math.hypot(a.points[1].x - a.points[0].x, a.points[1].y - a.points[0].y) < 3) {
        draw();
        return;
      }
      setShapes((prev) => [...prev, a]);
      setRedoStack([]);
    } else if (d.kind === "newsel") {
      const s = selRef.current;
      if (s && (s.w < 2 || s.h < 2)) {
        // 点击而非框选：若点在旧选区外且太小，取消
        setSel(null);
        selRef.current = null;
      }
    }
  }

  function onDoubleClick() {
    if (selRef.current && !tool) finish("copy");
  }

  // 文字工具
  function onClickTextToolPlace(e: React.MouseEvent) {
    if ((e.target as HTMLElement).closest(".toolbar")) return; // 点工具栏不误放置文字
    if (tool !== "text" || !selRef.current) return;
    const p = toPhys(e);
    if (!inside(p, selRef.current!)) return;
    setTextDraft({ at: p, value: "" });
  }

  function commitText() {
    if (textDraft && textDraft.value.trim()) {
      const s: Shape = {
        id: shapeId.current++,
        tool: "text",
        color,
        width: WIDTHS[widthIdx],
        points: [textDraft.at],
        text: textDraft.value,
      };
      setShapes((prev) => [...prev, s]);
      setRedoStack([]);
    }
    setTextDraft(null);
  }

  // ---------- 放大镜 ----------

  function updateMagnifier(e: React.PointerEvent, p: Pt) {
    const f = frameRef.current;
    const mag = magRef.current;
    if (!f || !mag) return;
    if (dragRef.current.kind !== "newsel" && dragRef.current.kind !== "resize") {
      hideMagnifier();
      return;
    }
    mag.style.display = "block";
    const mctx = mag.getContext("2d")!;
    const half = 8;
    const sx = Math.max(0, Math.min(p.x - half, f.w - half * 2));
    const sy = Math.max(0, Math.min(p.y - half, f.h - half * 2));
    mctx.imageSmoothingEnabled = false;
    mctx.clearRect(0, 0, mag.width, mag.height);
    mctx.drawImage(f.bmp, sx, sy, half * 2, half * 2, 0, 0, mag.width, mag.height);
    // 中心十字
    mctx.strokeStyle = "rgba(255,60,60,.9)";
    mctx.lineWidth = 1;
    const cx = mag.width / 2;
    const cy = mag.height / 2;
    mctx.beginPath();
    mctx.moveTo(cx, 0);
    mctx.lineTo(cx, mag.height);
    mctx.moveTo(0, cy);
    mctx.lineTo(mag.width, cy);
    mctx.stroke();
    const s = selRef.current;
    const label = s && dragRef.current.kind !== "newsel" ? `${s.w} × ${s.h}` : `x:${p.x} y:${p.y}`;
    mctx.fillStyle = "rgba(0,0,0,.75)";
    mctx.fillRect(0, mag.height - 22, mag.width, 22);
    mctx.fillStyle = "#fff";
    mctx.font = "13px Consolas, monospace";
    mctx.fillText(label, 6, mag.height - 6);
    // 定位：避开光标
    const mx = e.clientX + 24;
    const my = e.clientY + 24;
    mag.style.left = `${mx + mag.width > window.innerWidth ? e.clientX - mag.width - 16 : mx}px`;
    mag.style.top = `${my + mag.height > window.innerHeight ? e.clientY - mag.height - 16 : my}px`;
  }

  function hideMagnifier() {
    if (magRef.current) magRef.current.style.display = "none";
  }

  // ---------- 完成动作 ----------

  const finish = useCallback(
    async (action: "copy" | "save" | "pin", path?: string) => {
      const f = frameRef.current;
      const s = selRef.current;
      if (!f || !s || s.w < 2 || s.h < 2 || finishedRef.current) return;
      if (action === "save") {
        const picked = await save({
          defaultPath: timestampName(),
          filters: [{ name: "PNG 图片", extensions: ["png"] }],
        });
        if (!picked) return;
        path = picked;
      }
      finishedRef.current = true;
      try {
        const png = await compositePNG(f.bmp, annRef.current!, s);
        await invoke("finish_capture", packRequest({ action, monitor, x: s.x, y: s.y, path }, png), {
          headers: {},
        });
      } catch (err) {
        finishedRef.current = false;
        alert(`操作失败: ${err}`);
        return;
      }
      await invoke("overlay_finish");
    },
    [monitor],
  );

  const cancel = useCallback(async () => {
    await invoke("overlay_finish");
  }, []);

  const undo = () => {
    setShapes((prev) => {
      if (!prev.length) return prev;
      setRedoStack((r) => [...r, prev[prev.length - 1]]);
      return prev.slice(0, -1);
    });
  };
  const redo = () => {
    setRedoStack((r) => {
      if (!r.length) return r;
      setShapes((prev) => [...prev, r[r.length - 1]]);
      return r.slice(0, -1);
    });
  };

  // ---------- 键盘 ----------

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (textDraft) return; // 文字编辑中输入框自理
      if (e.key === "Escape") {
        e.preventDefault();
        cancel();
      } else if (e.key === "Enter" && selRef.current) {
        finish("copy");
      } else if (e.key.toLowerCase() === "c" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        if (selRef.current) finish("copy");
      } else if (e.key.toLowerCase() === "s" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        if (selRef.current) finish("save");
      } else if (e.key.toLowerCase() === "z" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        undo();
      } else if (e.key.toLowerCase() === "y" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        redo();
      } else if (e.key.toLowerCase() === "p" && !e.ctrlKey && !e.metaKey && selRef.current) {
        finish("pin");
      } else if (selRef.current && ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(e.key)) {
        e.preventDefault();
        const step = e.shiftKey ? 10 : 1;
        const s = { ...selRef.current };
        if (e.key === "ArrowUp") s.y -= step;
        if (e.key === "ArrowDown") s.y += step;
        if (e.key === "ArrowLeft") s.x -= step;
        if (e.key === "ArrowRight") s.x += step;
        selRef.current = s;
        setSel(s);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cancel, finish, textDraft]);

  // ---------- UI 定位 ----------

  const cssSel = sel
    ? { left: sel.x / dpr, top: sel.y / dpr, width: sel.w / dpr, height: sel.h / dpr }
    : null;
  const toolbarVisible = !!sel && hasFrame && !dragging && sel.w >= 2 && sel.h >= 2;
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  let tbStyle: React.CSSProperties | undefined;
  if (cssSel) {
    const TB_W = 580;
    const TB_H = 84;
    let top = cssSel.top + cssSel.height + 8;
    if (top + TB_H > vh) top = cssSel.top - TB_H - 8;
    if (top < 4) top = Math.min(cssSel.top + cssSel.height - TB_H - 6, vh - TB_H - 4);
    const left = Math.max(4, Math.min(cssSel.left + cssSel.width - TB_W, vw - TB_W - 4));
    tbStyle = { position: "fixed", left, top };
  }

  return (
    <div
      className="overlay-root"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onDoubleClick={onDoubleClick}
      onClick={onClickTextToolPlace}
    >
      <canvas ref={bgRef} className="layer" />
      <canvas ref={annRef} className="layer" />

      {/* 选区边框 + 尺寸标签（DOM 呈现，不进入导出图） */}
      {cssSel && (
        <>
          <div
            className="sel-border"
            style={{ ...cssSel, display: sel && sel.w > 0 ? "block" : "none" }}
          />
          <div className="sel-size" style={{ left: cssSel.left, top: cssSel.top - 26 }}>
            {sel?.w} × {sel?.h}
          </div>
          {sel && sel.w >= 2 && sel.h >= 2 &&
            Array.from({ length: 8 }).map((_, i) => {
              const hp = handlePos(sel, i);
              return (
                <div
                  key={i}
                  className="sel-handle"
                  style={{
                    left: hp.x / dpr - 4,
                    top: hp.y / dpr - 4,
                    cursor: CURSORS[i],
                  }}
                />
              );
            })}
        </>
      )}

      {/* 放大镜 */}
      <canvas ref={magRef} className="magnifier" width={176} height={198} />

      {/* 工具栏 */}
      {toolbarVisible && tbStyle && (
        <div className="toolbar" style={tbStyle} onPointerDown={(e) => e.stopPropagation()}>
          <div className="tb-row">
            <button
              className={`tb-btn ${tool === null ? "active" : ""}`}
              title="选区（拖拽调整）"
              onClick={() => setTool(null)}
            >
              <svg viewBox="0 0 24 24" className="ico">
                <rect x="3" y="3" width="12" height="12" strokeDasharray="3 2" />
                <path d="M19 9v10H9" />
              </svg>
            </button>
            {TOOL_DEFS.map((t) => (
              <button
                key={t.tool}
                className={`tb-btn ${tool === t.tool ? "active" : ""}`}
                title={t.label}
                onClick={() => setTool(t.tool === tool ? null : t.tool)}
              >
                <svg viewBox="0 0 24 24" className="ico" stroke="currentColor" fill="currentColor">
                  {t.icon}
                </svg>
              </button>
            ))}
            <span className="tb-sep" />
            {COLORS.map((c) => (
              <span
                key={c}
                className={`swatch ${c === color ? "active" : ""}`}
                style={{ background: c }}
                onClick={() => setColor(c)}
              />
            ))}
            <span className="tb-sep" />
            {WIDTHS.map((w, i) => (
              <span
                key={w}
                className={`wdot ${i === widthIdx ? "active" : ""}`}
                onClick={() => setWidthIdx(i)}
                title={`线宽 ${w}`}
              >
                <i style={{ width: Math.min(4 + i * 4, 16), height: Math.min(4 + i * 4, 16) }} />
              </span>
            ))}
          </div>
          <div className="tb-row">
            <button className="tb-btn" title="撤销 (Ctrl+Z)" onClick={undo} disabled={!shapes.length}>
              <svg viewBox="0 0 24 24" className="ico" stroke="currentColor">
                <path d="M9 7 4 12l5 5" fill="none" />
                <path d="M4 12h10a5 5 0 0 1 0 10h-2" fill="none" />
              </svg>
            </button>
            <button className="tb-btn" title="重做 (Ctrl+Y)" onClick={redo} disabled={!redoStack.length}>
              <svg viewBox="0 0 24 24" className="ico" stroke="currentColor">
                <path d="M15 7l5 5-5 5" fill="none" />
                <path d="M20 12H10a5 5 0 0 0 0 10h2" fill="none" />
              </svg>
            </button>
            <span className="tb-spacer" />
            <button className="tb-btn" title="复制到剪贴板 (Ctrl+C / 双击完成)" onClick={() => finish("copy")}>
              <svg viewBox="0 0 24 24" className="ico" stroke="currentColor">
                <rect x="9" y="9" width="11" height="11" rx="2" />
                <path d="M16 9V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h3" fill="none" />
              </svg>
            </button>
            <button className="tb-btn" title="保存为文件 (Ctrl+S)" onClick={() => finish("save")}>
              <svg viewBox="0 0 24 24" className="ico" stroke="currentColor">
                <path d="M5 4h11l3 3v13H5z" fill="none" />
                <path d="M8 4v5h7V4" fill="none" />
                <rect x="8" y="13" width="8" height="6" />
              </svg>
            </button>
            <button className="tb-btn accent" title="贴图到桌面 (P)" onClick={() => finish("pin")}>
              <svg viewBox="0 0 24 24" className="ico" stroke="currentColor">
                <path d="M8 3h8v7l2 2H6l2-2z" fill="none" />
                <line x1="12" y1="12" x2="12" y2="21" />
              </svg>
            </button>
            <button className="tb-btn danger" title="取消 (Esc)" onClick={cancel}>
              <svg viewBox="0 0 24 24" className="ico" stroke="currentColor">
                <line x1="6" y1="6" x2="18" y2="18" />
                <line x1="18" y1="6" x2="6" y2="18" />
              </svg>
            </button>
          </div>
        </div>
      )}

      {/* 文字草稿输入框 */}
      {textDraft && (
        <textarea
          className="text-draft"
          autoFocus
          value={textDraft.value}
          style={{
            left: textDraft.at.x / dpr,
            top: textDraft.at.y / dpr,
            color,
            fontSize: (8 + WIDTHS[widthIdx] * 3) / dpr,
          }}
          placeholder="输入文字…"
          onChange={(e) => setTextDraft({ ...textDraft, value: e.target.value })}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              commitText();
            } else if (e.key === "Escape") {
              e.preventDefault();
              setTextDraft(null);
            }
          }}
          onBlur={commitText}
        />
      )}

      {!hasFrame && <div className="overlay-wait" />}
    </div>
  );
}
