import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

// —— JS 错误管道：把渲染端异常转发到 Rust 标准错误输出（dev 终端可见）。
// 直接用 __TAURI_INTERNALS__ 以避免再依赖 App 模块（它可能正是坏的一环）。
function forward(level: string, msg: string) {
  // 注意：不要用 document.title 做信标——tauri 对显式设置过 title 的窗口
  // 会经 wry 同步 SetTitle，实测会与窗口创建竞争导致窗口异常消失
  try {
    const internals = (window as any).__TAURI_INTERNALS__;
    if (internals?.invoke) {
      const p = internals.invoke("debug_log", { level, msg });
      p?.catch?.(() => {});
    }
  } catch {
    /* 忽略转发失败 */
  }
}

window.onerror = (message, source, line, col, err) => {
  forward("error", `${message} @ ${source}:${line}:${col} ${err?.stack ?? ""}`);
};
window.addEventListener("unhandledrejection", (e) => {
  forward("reject", String((e as PromiseRejectionEvent).reason?.stack ?? (e as PromiseRejectionEvent).reason));
});
window.addEventListener("error", (e) => {
  const t = e.target as HTMLElement;
  if (t && (t.tagName === "SCRIPT" || t.tagName === "LINK")) {
    forward("asset", `资源加载失败: ${t.getAttribute("src") ?? t.getAttribute("href")}`);
  }
}, true);
forward("boot", `JS 启动 view=${new URLSearchParams(window.location.search).get("view")}`);

// 不使用 StrictMode：截图流程是命令式 canvas 管线，
// 双调用 effect 会造成重复拉帧/回执。
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(<App />);
