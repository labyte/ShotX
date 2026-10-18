//! DXGI Desktop Duplication 捕获后端。
//!
//! 从显卡合成层取帧（IDXGIOutputDuplication），不经过 GDI 打印路径，
//! 因此不受 BitBlt/PrintWindow 类钩子影响，可与挂钩型终端软件共存。
//! 关键语义：
//! - DuplicateOutput 后第一次 AcquireNextFrame 立即返回当前桌面快照，
//!   所以 one-shot 截图只需一次取帧；
//! - DXGI_ERROR_WAIT_TIMEOUT 表示屏幕无新帧，重试即可，不是错误；
//! - DXGI_ERROR_ACCESS_LOST（分辨率切换/锁屏/显示拓扑变化）需重建 duplication。
//! 混合显卡笔记本要求 D3D device 建在 output 所属 adapter 上，这里按
//! adapter 缓存 device。

use std::sync::{Mutex, OnceLock};

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_USAGE_STAGING, ID3D11Device, ID3D11DeviceContext,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
    DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_NOT_FOUND,
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIOutput;
use windows::Win32::Graphics::Dxgi::IDXGIAdapter1;
use windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE2D_DESC;

use super::{bgra_to_rgba, MonitorInfo, RawFrame};

fn utf16_z(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// 每个 DXGI 输出的持久状态
struct OutputSlot {
    name: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    adapter_index: u32,
    output1: IDXGIOutput1,
    duplication: Option<IDXGIOutputDuplication>,
    staging: Option<ID3D11Texture2D>,
    staging_w: u32,
    staging_h: u32,
}

struct Engine {
    factory: IDXGIFactory2,
    /// adapter_index -> D3D device/context（DuplicateOutput 要求 device 建在对应 adapter 上）
    devices: Vec<Option<(ID3D11Device, ID3D11DeviceContext)>>,
    slots: Vec<OutputSlot>,
}

static ENGINE: OnceLock<Mutex<Option<Engine>>> = OnceLock::new();

impl Engine {
    fn new() -> Result<Self, String> {
        unsafe {
            let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))
                .map_err(|e| format!("CreateDXGIFactory2 失败: {e}"))?;
            let mut engine = Engine {
                factory,
                devices: Vec::new(),
                slots: Vec::new(),
            };
            engine.rebuild_outputs()?;
            Ok(engine)
        }
    }

    /// 重新枚举 DXGI 输出（显示拓扑变化时调用），device 缓存按 adapter 序号对齐
    fn rebuild_outputs(&mut self) -> Result<(), String> {
        let mut slots: Vec<OutputSlot> = Vec::new();
        let mut adapter_count = 0u32;
        unsafe {
            for i in 0.. {
                let adapter: IDXGIAdapter1 = match self.factory.EnumAdapters1(i) {
                    Ok(a) => a,
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(e) => return Err(format!("EnumAdapters1 失败: {e}")),
                };
                adapter_count = adapter_count.max(i + 1);
                for j in 0.. {
                    let output: IDXGIOutput = match adapter.EnumOutputs(j) {
                        Ok(o) => o,
                        Err(_) => break,
                    };
                    let desc = output.GetDesc().map_err(|e| format!("GetDesc 失败: {e}"))?;
                    if !desc.AttachedToDesktop.as_bool() {
                        continue;
                    }
                    let rc = desc.DesktopCoordinates;
                    let output1: IDXGIOutput1 =
                        output.cast().map_err(|e| format!("cast IDXGIOutput1 失败: {e}"))?;
                    slots.push(OutputSlot {
                        name: utf16_z(&desc.DeviceName),
                        x: rc.left,
                        y: rc.top,
                        width: (rc.right - rc.left).max(0) as u32,
                        height: (rc.bottom - rc.top).max(0) as u32,
                        adapter_index: i,
                        output1,
                        duplication: None,
                        staging: None,
                        staging_w: 0,
                        staging_h: 0,
                    });
                }
            }
        }
        self.devices.resize_with(adapter_count as usize, || None);
        self.slots = slots;
        Ok(())
    }

    fn device_for(&mut self, adapter_index: u32) -> Result<(ID3D11Device, ID3D11DeviceContext), String> {
        if let Some(d) = &self.devices.get(adapter_index as usize).cloned().flatten() {
            return Ok(d.clone());
        }
        unsafe {
            let adapter = self
                .factory
                .EnumAdapters1(adapter_index)
                .map_err(|e| format!("EnumAdapters1({adapter_index}) 失败: {e}"))?;
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                None,
                7, // D3D11_SDK_VERSION
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("D3D11CreateDevice 失败: {e}"))?;
            let pair = (
                device.ok_or("device 为空")?,
                context.ok_or("context 为空")?,
            );
            if let Some(slot) = self.devices.get_mut(adapter_index as usize) {
                *slot = Some(pair.clone());
            }
            Ok(pair)
        }
    }

    fn ensure_duplication(slot: &mut OutputSlot, device: &ID3D11Device) -> Result<(), String> {
        if slot.duplication.is_some() {
            return Ok(());
        }
        unsafe {
            let dup = slot
                .output1
                .DuplicateOutput(device)
                .map_err(|e| format!("DuplicateOutput({}) 失败: {e}", slot.name))?;
            slot.duplication = Some(dup);
        }
        Ok(())
    }

    fn ensure_staging(slot: &mut OutputSlot, device: &ID3D11Device) -> Result<ID3D11Texture2D, String> {
        let same = slot.staging.is_some()
            && slot.staging_w == slot.width
            && slot.staging_h == slot.height;
        if same {
            if let Some(t) = &slot.staging {
                return Ok(t.clone());
            }
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: slot.width,
            Height: slot.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        unsafe {
            let mut tex: Option<ID3D11Texture2D> = None;
            device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .map_err(|e| format!("CreateTexture2D 失败: {e}"))?;
            let tex = tex.ok_or("纹理为空")?;
            slot.staging = Some(tex.clone());
            slot.staging_w = slot.width;
            slot.staging_h = slot.height;
            Ok(tex)
        }
    }

    /// 截取单个显示器（含 ACCESS_LOST 重建重试）
    fn capture_slot(slot: &mut OutputSlot, device: &ID3D11Device, ctx: &ID3D11DeviceContext) -> Result<RawFrame, String> {
        Self::ensure_duplication(slot, device)?;
        // 诊断：记录桌面合成格式（HDR 开启时为 FP16，非 BGRA8 会导致拷贝失败）
        if let Some(dup) = &slot.duplication {
            let d = unsafe { dup.GetDesc() };
            crate::diag::log(&format!(
                "duplication {} 桌面格式={:?} 模式={}x{} refresh={}x{}",
                slot.name,
                d.ModeDesc.Format.0,
                d.ModeDesc.Width,
                d.ModeDesc.Height,
                d.ModeDesc.RefreshRate.Numerator,
                d.ModeDesc.RefreshRate.Denominator
            ));
        }
        let staging = Self::ensure_staging(slot, device)?;

        for attempt in 0..4u32 {
            let dup = slot.duplication.clone().ok_or("无 duplication")?;
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            // 首次调用会立即返回当前桌面；屏幕静止时超时也正常，等待下一轮
            let timeout = if attempt == 0 { 120 } else { 300 };
            let acquired = unsafe { dup.AcquireNextFrame(timeout, &mut info, &mut resource) };
            match acquired {
                Ok(()) => {
                    let result = (|| -> Result<RawFrame, String> {
                        unsafe {
                            let res = resource.ok_or("无桌面资源")?;
                            let tex: ID3D11Texture2D =
                                res.cast().map_err(|e| format!("cast 纹理失败: {e}"))?;
                            // 必须在 ReleaseFrame 前把帧拷到 staging
                            ctx.CopyResource(&staging, &tex);
                            dup.ReleaseFrame().map_err(|e| format!("ReleaseFrame 失败: {e}"))?;
                            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                            ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                                .map_err(|e| format!("Map 失败: {e}"))?;
                            let row_bytes = (slot.width * 4) as usize;
                            let pitch = mapped.RowPitch as usize;
                            let mut buf = vec![0u8; row_bytes * slot.height as usize];
                            let src = mapped.pData as *const u8;
                            for y in 0..slot.height as usize {
                                let s = src.add(y * pitch);
                                buf[y * row_bytes..(y + 1) * row_bytes]
                                    .copy_from_slice(std::slice::from_raw_parts(s, row_bytes));
                            }
                            ctx.Unmap(&staging, 0);
                            bgra_to_rgba(&mut buf);
                            Ok(RawFrame { width: slot.width, height: slot.height, rgba: buf })
                        }
                    })();
                    if result.is_ok() {
                        // 每次捕获后丢弃 duplication：DuplicateOutput 刚创建时的首次
                        // AcquireNextFrame 必定立即返回当前桌面，避免屏幕静止时
                        // 复用旧 duplication 拿不到新帧而超时。
                        slot.duplication = None;
                    }
                    return result;
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    continue;
                }
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    // 显示模式/拓扑变化：重建 duplication 再试一轮
                    slot.duplication = None;
                    Self::ensure_duplication(slot, device)?;
                    continue;
                }
                Err(e) => return Err(format!("AcquireNextFrame 失败: {e}")),
            }
        }
        Err("获取桌面帧超时".into())
    }
}

/// 捕获所有显示器：把 GDI 枚举的监视器按名称匹配到 DXGI 输出
pub fn capture_all(monitors: &[MonitorInfo]) -> Result<Vec<(MonitorInfo, RawFrame)>, String> {
    let mutex = ENGINE.get_or_init(|| Mutex::new(None));
    let mut guard = mutex.lock().unwrap_or_else(|p| p.into_inner());
    if guard.is_none() {
        *guard = Some(Engine::new()?);
    }
    let engine = guard.as_mut().ok_or("引擎未初始化")?;

    // 拓扑可能已变化：若输出数量/位置与请求不符，先重建一次
    let mut want_rebuild = engine.slots.len() != monitors.len();
    for m in monitors {
        if !engine
            .slots
            .iter()
            .any(|s| s.name == m.name || (s.x == m.x && s.y == m.y && s.width == m.width && s.height == m.height))
        {
            want_rebuild = true;
            break;
        }
    }
    if want_rebuild {
        engine.rebuild_outputs()?;
    }

    let mut out = Vec::with_capacity(monitors.len());
    for m in monitors {
        let pos = engine
            .slots
            .iter()
            .position(|s| s.name == m.name)
            .or_else(|| {
                engine
                    .slots
                    .iter()
                    .position(|s| s.x == m.x && s.y == m.y && s.width == m.width && s.height == m.height)
            })
            .ok_or_else(|| format!("显示器 {} 无 DXGI 输出", m.name))?;
        let (device, ctx) = engine.device_for(engine.slots[pos].adapter_index)?;
        let frame = Engine::capture_slot(&mut engine.slots[pos], &device, &ctx)?;
        out.push((m.clone(), frame));
    }
    Ok(out)
}
