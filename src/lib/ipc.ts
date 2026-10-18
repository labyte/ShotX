import { invoke } from "@tauri-apps/api/core";

export { invoke };
export { listen } from "@tauri-apps/api/event";

/** 打包 JS → Rust 的原始请求：[u32 元数据长度 LE][元数据 JSON][二进制] */
export function packRequest(meta: unknown, bytes: Uint8Array): Uint8Array {
  const json = new TextEncoder().encode(JSON.stringify(meta));
  const out = new Uint8Array(4 + json.length + bytes.length);
  new DataView(out.buffer).setUint32(0, json.length, true);
  out.set(json, 4);
  out.set(bytes, 4 + json.length);
  return out;
}

export interface HistoryItem {
  id: string;
  created: number;
  width: number;
  height: number;
  file: string;
}

export interface AppSettings {
  captureHotkey: string;
  pinsHotkey: string;
  autostart: boolean;
  historyLimit: number;
  hideTaskbar: boolean;
  hidePinsOnCapture: boolean;
}

/** ArrayBuffer 响应 → 位图。头 8 字节为 u32 LE 宽高（RGBA 帧）*/
export async function frameBitmap(
  buf: ArrayBuffer,
): Promise<{ bmp: ImageBitmap; w: number; h: number } | null> {
  if (buf.byteLength < 8) return null;
  const dv = new DataView(buf);
  const w = dv.getUint32(0, true);
  const h = dv.getUint32(4, true);
  const rgba = new Uint8ClampedArray(buf.slice(8));
  const bmp = await createImageBitmap(new ImageData(rgba, w, h));
  return { bmp, w, h };
}

/** ArrayBuffer 中的 PNG → 位图 */
export async function pngBitmap(buf: ArrayBuffer): Promise<ImageBitmap | null> {
  if (buf.byteLength < 8) return null;
  const blob = new Blob([buf], { type: "image/png" });
  return await createImageBitmap(blob);
}

export function timestampName(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `ShotX-${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}.png`;
}
