# ShotX

类 Snipaste 的轻量截图贴图工具，基于 Tauri v2 + React + TypeScript。
全程驻留系统托盘，**没有任何主窗口**，常驻内存占用极小。

## 功能

| 功能 | 操作 |
|---|---|
| 截图 | 全局快捷键 `F1`（可自定义），或左键单击托盘图标 |
| 选区 | 拖拽框选；8 向把手微调；方向键 1px / Shift+方向键 10px 移动；放大镜辅助取色定位 |
| 标注 | 矩形 / 椭圆 / 直线 / 箭头 / 画笔 / 马赛克 / 文字（支持中文输入法），9 色 × 4 档线宽，撤销/重做 |
| 完成 | `Ctrl+C`/双击/`Enter` 复制；`Ctrl+S` 保存文件；`P` 贴图置顶；`Esc` 取消 |
| 贴图 | 无边框置顶；拖动移窗；滚轮缩放 5%–1600%；右键菜单：复制/另存/透明度/置顶切换/实际大小/关闭；`F3` 全局显隐所有贴图 |
| 历史 | 每次完成动作自动入库（可设保留数量），设置窗口内缩略图墙：双击贴图、复制、在文件夹显示、删除、清空 |
| 设置 | 托盘菜单"设置与历史"：快捷键自定义（即改即生效）、开机自启、截图时隐藏任务栏/贴图、历史保留数 |

## 截屏原理（重点）

三级后端自动降级，全部为用户态系统 API：

1. **WGC（Windows.Graphics.Capture）**——主后端。从 DWM 合成器取每个显示器的
   提交帧。本机实测：IPGuard 类终端软件会置黑 DXGI duplication 的帧，
   而 WGC 不受影响；且 WGC 不占用 duplication 名额，与录屏软件互不冲突。
2. **DXGI Desktop Duplication**（`IDXGIOutputDuplication`）——从显卡合成层取帧，
   不经 GDI 打印路径；WGC 不可用时启用。
3. **GDI BitBlt**——远程桌面等极端环境的最后兜底。

> 诚实边界：驱动级屏幕过滤、受 DRM/HDCP 保护的内容、设置了
> `WDA_EXCLUDEFROMCAPTURE` 的窗口，在任何用户态 API 下都会是黑块——
> 这不是本工具"规避"的目标，也规避不了。

其他底层要点：
- 进程声明 Per-Monitor-V2 DPI 感知，捕获/窗口几何全程物理像素，混合 DPI 多屏不漂移
- 每显示器一个预创建隐藏覆盖层窗口；截图时序为"隐藏→捕获→渲染→回执→统一显示"，永不把自己拍进截图
- 帧数据经 Tauri v2 类型化 IPC 原始二进制通道（ArrayBuffer）直传前端，无 PNG 编解码损耗

## 开发

```bash
npm install
npm run tauri dev      # 开发运行
npm run tauri build    # 发布打包（NSIS 安装器 / 便携 exe）

# 端到端冒烟测试（另开终端，需 dev 实例在跑）
cargo run --manifest-path src-tauri/Cargo.toml --example drive
```

目录结构：

```
src/            React 前端（单入口，?view= 区分 overlay / pin / settings 三种窗口）
src-tauri/src/
  capture/      截屏后端：dxgi.rs（主）/ gdi.rs（兜底）
  overlay.rs    截图会话状态机 + 覆盖层窗口编排
  pin.rs        贴图窗口（建窗/缩放数学/分层窗口透明度）
  commands.rs   IPC 命令面（帧传输、完成动作、历史、设置）
  history.rs    历史落盘 + 缩略图 + 保留数裁剪
  tray.rs / hotkey.rs / settings.rs / winctl.rs / winutil.rs
```

## 已知限制

- 贴图不跟随切换 Windows 虚拟桌面（Win32 无公开的跨虚拟桌面置顶 API）
- 屏幕静止时 DXGI 不产生新帧，采用"每次截图新建 duplication"策略拿当前桌面（首帧语义保证即时性）
- 跨显示器拖拽框选暂未支持（每屏独立选区，与多数快捷键截图工具一致）
