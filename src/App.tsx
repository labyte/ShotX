import { lazy, Suspense } from "react";

const Overlay = lazy(() => import("./views/Overlay"));
const Pin = lazy(() => import("./views/Pin"));
const Settings = lazy(() => import("./views/Settings"));

/** 单入口多视图：窗口 URL 的 ?view= 参数决定渲染哪一个 */
function App() {
  const params = new URLSearchParams(window.location.search);
  const view = params.get("view") ?? "settings";

  if (view === "pin") {
    // 贴图窗口需要真透明背景
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
  }

  return (
    <Suspense fallback={null}>
      {view === "overlay" && <Overlay monitor={Number(params.get("m") ?? 0)} />}
      {view === "pin" && <Pin id={params.get("id") ?? ""} />}
      {view === "settings" && <Settings />}
    </Suspense>
  );
}

export default App;
