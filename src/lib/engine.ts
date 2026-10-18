/** 标注引擎：形状模型与全量重放渲染（坐标一律物理像素） */

export type Tool = "rect" | "ellipse" | "line" | "arrow" | "pen" | "mosaic" | "text";

export interface Pt {
  x: number;
  y: number;
}

export interface Sel {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Shape {
  id: number;
  tool: Tool;
  color: string;
  /** 线宽档位（物理像素），同时驱动字号/马赛克笔径 */
  width: number;
  /** rect/ellipse/line/arrow: [起点,终点]；pen/mosaic: 轨迹；text: [锚点] */
  points: Pt[];
  text?: string;
}

export const COLORS = ["#e51400", "#ff8c00", "#ffd600", "#60a917", "#00b4ff", "#0078d7", "#7a3cc8", "#ffffff", "#111111"];
export const WIDTHS = [2, 4, 7, 12];

/** 文本字号由线宽档位推导 */
export function fontSizeOf(width: number): number {
  return Math.round(8 + width * 3);
}

/** 马赛克笔刷直径 */
export function brushOf(width: number): number {
  return 10 + width * 5;
}

/** 线宽/字号在物理像素下偏细，整体放缩到与屏幕观感一致 */
export function scaled(width: number, dpr: number): number {
  return Math.max(1, Math.round(width * dpr));
}

/** 预生成像素化底图（马赛克用，一次生成、逐形状复用） */
export function buildPixelated(
  src: CanvasImageSource,
  w: number,
  h: number,
  block: number,
): HTMLCanvasElement {
  const out = document.createElement("canvas");
  out.width = w;
  out.height = h;
  const ctx = out.getContext("2d")!;
  const small = document.createElement("canvas");
  small.width = Math.max(1, Math.floor(w / block));
  small.height = Math.max(1, Math.floor(h / block));
  const sctx = small.getContext("2d")!;
  sctx.drawImage(src, 0, 0, small.width, small.height);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(small, 0, 0, w, h);
  return out;
}

export function normRect(s: Shape): Sel {
  const a = s.points[0];
  const b = s.points[s.points.length - 1];
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    w: Math.abs(a.x - b.x),
    h: Math.abs(a.y - b.y),
  };
}

function tracePath(ctx: CanvasRenderingContext2D, pts: Pt[]) {
  ctx.beginPath();
  ctx.moveTo(pts[0].x, pts[0].y);
  for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i].x, pts[i].y);
}

function drawArrow(ctx: CanvasRenderingContext2D, s: Shape, lw: number) {
  const a = s.points[0];
  const b = s.points[s.points.length - 1];
  const angle = Math.atan2(b.y - a.y, b.x - a.x);
  const head = Math.max(lw * 4, Math.min(14 * (lw / 2), Math.hypot(b.x - a.x, b.y - a.y) * 0.5));
  ctx.beginPath();
  ctx.moveTo(a.x, a.y);
  ctx.lineTo(b.x - head * 0.6 * Math.cos(angle), b.y - head * 0.6 * Math.sin(angle));
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(b.x, b.y);
  ctx.lineTo(b.x - head * Math.cos(angle - Math.PI / 7), b.y - head * Math.sin(angle - Math.PI / 7));
  ctx.lineTo(b.x - head * 0.65 * Math.cos(angle), b.y - head * 0.65 * Math.sin(angle));
  ctx.lineTo(b.x - head * Math.cos(angle + Math.PI / 7), b.y - head * Math.sin(angle + Math.PI / 7));
  ctx.closePath();
  ctx.fill();
}

function drawOne(
  ctx: CanvasRenderingContext2D,
  s: Shape,
  pixel: HTMLCanvasElement | null,
  dpr: number,
) {
  const lw = scaled(s.width, dpr);
  ctx.strokeStyle = s.color;
  ctx.fillStyle = s.color;
  ctx.lineWidth = lw;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";

  switch (s.tool) {
    case "rect": {
      const r = normRect(s);
      ctx.strokeRect(r.x, r.y, r.w, r.h);
      break;
    }
    case "ellipse": {
      const r = normRect(s);
      ctx.beginPath();
      ctx.ellipse(r.x + r.w / 2, r.y + r.h / 2, r.w / 2, r.h / 2, 0, 0, Math.PI * 2);
      ctx.stroke();
      break;
    }
    case "line":
      tracePath(ctx, s.points);
      ctx.stroke();
      break;
    case "arrow":
      drawArrow(ctx, s, lw);
      break;
    case "pen":
      if (s.points.length > 1) {
        tracePath(ctx, s.points);
        ctx.stroke();
      } else if (s.points.length === 1) {
        ctx.beginPath();
        ctx.arc(s.points[0].x, s.points[0].y, lw / 2, 0, Math.PI * 2);
        ctx.fill();
      }
      break;
    case "mosaic": {
      if (!pixel || s.points.length === 0) break;
      ctx.save();
      // 先用笔迹造出遮罩，再用 source-atop 把像素化底图只落在遮罩上
      ctx.lineWidth = scaled(brushOf(s.width), dpr);
      tracePath(ctx, s.points);
      if (s.points.length === 1) {
        ctx.beginPath();
        ctx.arc(s.points[0].x, s.points[0].y, ctx.lineWidth / 2, 0, Math.PI * 2);
        ctx.fill();
      } else {
        ctx.strokeStyle = "#000";
        ctx.stroke();
      }
      ctx.globalCompositeOperation = "source-atop";
      ctx.drawImage(pixel, 0, 0);
      ctx.restore();
      break;
    }
    case "text": {
      const size = scaled(fontSizeOf(s.width), dpr);
      ctx.font = `${size}px "Microsoft YaHei", "Segoe UI", sans-serif`;
      ctx.textBaseline = "top";
      const p = s.points[0];
      const lines = (s.text ?? "").split("\n");
      lines.forEach((ln, i) => {
        ctx.fillText(ln, p.x, p.y + i * (size * 1.35));
      });
      break;
    }
  }
}

/** 全量重放：清空 → 裁剪到选区 → 画所有形状（含正在拖拽的预览） */
export function renderAnnotations(
  ctx: CanvasRenderingContext2D,
  shapes: Shape[],
  active: Shape | null,
  sel: Sel,
  pixel: HTMLCanvasElement | null,
  dpr: number,
) {
  const c = ctx.canvas;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, c.width, c.height);
  ctx.save();
  ctx.beginPath();
  ctx.rect(sel.x, sel.y, sel.w, sel.h);
  ctx.clip();
  for (const s of shapes) drawOne(ctx, s, pixel, dpr);
  if (active) drawOne(ctx, active, pixel, dpr);
  ctx.restore();
}

/** 导出合成图：选区内 原图 + 标注 */
export async function compositePNG(
  frame: ImageBitmap,
  ann: HTMLCanvasElement,
  sel: Sel,
): Promise<Uint8Array> {
  const out = document.createElement("canvas");
  out.width = sel.w;
  out.height = sel.h;
  const ctx = out.getContext("2d")!;
  ctx.drawImage(frame, sel.x, sel.y, sel.w, sel.h, 0, 0, sel.w, sel.h);
  ctx.drawImage(ann, sel.x, sel.y, sel.w, sel.h, 0, 0, sel.w, sel.h);
  const blob = await new Promise<Blob>((res) => out.toBlob((b) => res(b!), "image/png"));
  return new Uint8Array(await blob.arrayBuffer());
}
