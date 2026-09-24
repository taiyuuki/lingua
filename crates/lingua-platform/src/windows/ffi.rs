//! Windows UI Automation / SendInput / GDI 最小封装。
//!
//! 与 `macos/ffi.rs` 对称：本文件是 Windows 平台唯一允许 `unsafe`
//! 的区域（workspace lints 对其它 crate 一律 deny）。
//!
//! 权限说明：Windows 没有 macOS 式的「辅助功能」开关 ——
//! UI Automation 与 SendInput 开箱可用。唯一边界是 UIPI：
//! 普通权限运行的 Lingua 读不到管理员权限运行的窗口，
//! 此时表现为拿不到选区（fallback 也同样受限），最终按
//! `NoSelection` 温和报「未选中文本」，不需要权限引导。
#![allow(unsafe_code)]

use ::windows::Win32::Foundation::{HWND, POINT};
use ::windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC,
    MONITOR_DEFAULTTONEAREST, MonitorFromPoint, ROP_CODE, ReleaseDC, SRCCOPY, SelectObject,
};
use ::windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use ::windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern, UIA_TextPatternId,
};
use ::windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetCursorPos, GetForegroundWindow,
};
use ::windows::core::HRESULT;
use lingua_core::{AppError, AppResult, OcrImage};
use std::ffi::c_void;
use std::time::{Duration, Instant};

/// `RPC_E_CHANGED_MODE`：线程已用其它公寓模型初始化过 COM。
const RPC_E_CHANGED_MODE: HRESULT = HRESULT(0x8001_0106u32 as i32);

/// 字母 C 的虚拟键码（windows crate 不提供字母键常量）。
const VK_C: VIRTUAL_KEY = VIRTUAL_KEY(0x43);

// MARK: - 光标位置

/// 当前光标的全局坐标（逻辑像素，屏幕左上角为原点）。
///
/// `GetCursorPos` 返回物理像素，按光标所在显示器的有效 DPI
/// 换算成逻辑值，与 macOS（直接返回全局点）语义对齐。
pub fn cursor_position() -> Option<(f64, f64)> {
    unsafe {
        let mut point = POINT::default();
        GetCursorPos(&mut point).ok()?;
        let scale = monitor_scale_at(point);
        Some((f64::from(point.x) / scale, f64::from(point.y) / scale))
    }
}

/// 物理坐标所在显示器的缩放比（96 DPI = 1.0）；取不到时按 1.0。
fn monitor_scale_at(point: POINT) -> f64 {
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut dpi_x = 0u32;
        let mut dpi_y = 0u32;
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok() {
            f64::from(dpi_x.max(dpi_y)) / 96.0
        } else {
            1.0
        }
    }
}

// MARK: - 前台焦点判定

/// 窗口（含其子窗口树，如 WebView2 宿主）当前是否持有前台焦点。
///
/// Windows 上 WebView2 是独立的子 HWND 树：用户点击弹窗内容时，
/// 真实焦点会从 Tauri 主窗口转移给子窗口，主窗口随之收到
/// `Focused(false)` 事件——这不是「点到外部」。用前台窗口的根
/// 祖先是否等于目标窗口来裁决；焦点仍在目标窗口树内时返回 `true`。
pub fn window_in_foreground(hwnd: isize) -> bool {
    unsafe {
        let target = HWND(hwnd as *mut c_void);
        let foreground = GetForegroundWindow();
        if foreground.is_invalid() {
            return false;
        }
        GetAncestor(foreground, GA_ROOT) == target
    }
}

// MARK: - 按键模拟

fn key_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// 任一修饰键（Alt/Ctrl/Shift/Win，不分左右）当前是否被物理按住。
fn any_modifier_down() -> bool {
    const MODIFIER_VKS: [i32; 5] = [
        VK_MENU.0 as i32,
        VK_CONTROL.0 as i32,
        VK_SHIFT.0 as i32,
        VK_LWIN.0 as i32,
        VK_RWIN.0 as i32,
    ];
    MODIFIER_VKS.iter().any(|&vk| {
        let state = unsafe { GetAsyncKeyState(vk) };
        // 高位为 1 = 按下（低位是「自上次调用后是否按过」，跨线程不可靠，不用）。
        (state as u16 & 0x8000) != 0
    })
}

/// 等待触发快捷键的修饰键物理释放（上限 500ms）。
///
/// 全局快捷键在按下的瞬间触发：此刻用户往往还按着 Alt（alt+t）。
/// SendInput 的修饰键与物理按键状态是**叠加**关系，直接模拟 Ctrl+C
/// 会变成 Alt+Ctrl+C，目标应用不会执行复制（macOS 的 CGEventPost
/// 携带完整修饰键掩码、按事件本身解释，无此问题）。
fn wait_for_modifier_release() {
    const TIMEOUT: Duration = Duration::from_millis(500);
    const POLL: Duration = Duration::from_millis(15);
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if !any_modifier_down() {
            return;
        }
        std::thread::sleep(POLL);
    }
    tracing::debug!("modifier keys still held after timeout; sending copy anyway");
}

/// 向当前前台窗口发送一次 Ctrl+C（按下 + 抬起）。
///
/// 模拟输入受 UIPI 限制：目标以管理员权限运行时会被系统丢弃，
/// 表现为剪贴板不变，由轮询超时兜底（`NoSelection`）。
pub fn post_copy_shortcut() {
    wait_for_modifier_release();
    let events = [
        key_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_C, KEYBD_EVENT_FLAGS(0)),
    ];
    let releases = [
        key_input(VK_C, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    unsafe {
        SendInput(&events, std::mem::size_of::<INPUT>() as i32);
        // 按下与抬起之间留一小段间隔，给目标应用处理按键的时间。
        std::thread::sleep(Duration::from_millis(30));
        SendInput(&releases, std::mem::size_of::<INPUT>() as i32);
    }
}

// MARK: - UI Automation 划词

/// 通过 UI Automation（TextPattern）读取当前选中文本。
///
/// 失败语义与 macOS AX 实现对齐（SelectionService 据此决定 fallback）：
/// * COM 基础设施初始化失败 → [`AppError::Platform`]；
/// * 拿不到聚焦元素 / 不支持 TextPattern / 没有选区 / UIPI 受限 →
///   [`AppError::NoSelection`]（让上层尝试剪贴板 fallback）。
pub fn uia_capture_selected_text() -> AppResult<String> {
    unsafe {
        // 微软推荐 UIA 客户端用 MTA；线程池线程可能已按其它模型初始化，
        // 此时（RPC_E_CHANGED_MODE）沿用现状继续调用，只是不重复解除。
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        let must_uninitialize = hr.is_ok();
        if !must_uninitialize && hr != RPC_E_CHANGED_MODE {
            return Err(AppError::Platform(format!("CoInitializeEx failed: {hr}")));
        }
        let result = uia_read_selection();
        if must_uninitialize {
            CoUninitialize();
        }
        result
    }
}

unsafe fn uia_read_selection() -> AppResult<String> {
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(|error| {
                AppError::Platform(format!("failed to create CUIAutomation: {error}"))
            })?;

        let focused: IUIAutomationElement = match automation.GetFocusedElement() {
            Ok(element) => element,
            Err(error) => {
                tracing::debug!(error = %error, "UIA: focused element unavailable");
                return Err(AppError::NoSelection);
            }
        };

        let pattern: IUIAutomationTextPattern = match focused.GetCurrentPatternAs(UIA_TextPatternId)
        {
            Ok(pattern) => pattern,
            Err(error) => {
                tracing::debug!(error = %error, "UIA: no TextPattern on focused element");
                return Err(AppError::NoSelection);
            }
        };

        let ranges = match pattern.GetSelection() {
            Ok(ranges) => ranges,
            Err(error) => {
                tracing::debug!(error = %error, "UIA: GetSelection failed");
                return Err(AppError::NoSelection);
            }
        };

        let count = ranges.Length().unwrap_or(0);
        if count <= 0 {
            return Err(AppError::NoSelection);
        }
        let range = match ranges.GetElement(0) {
            Ok(range) => range,
            Err(error) => {
                tracing::debug!(error = %error, "UIA: empty selection range");
                return Err(AppError::NoSelection);
            }
        };

        // GetText 返回 BSTR（drop 时自动释放）；空串视为无选区。
        let text = range
            .GetText(-1)
            .map(|value| value.to_string())
            .unwrap_or_default();
        Ok(text)
    }
}

// MARK: - GDI 截图

/// 32bpp 顶层 DIB 是 BGRA 序，且 GDI 不保证 alpha —— 转 RGBA 并
/// 强制不透明，否则 PNG 在 webview 里会渲染成透明。
fn bgra_to_opaque_rgba(buffer: &mut [u8]) {
    for pixel in buffer.chunks_exact_mut(4) {
        pixel.swap(0, 2); // B <-> R
        pixel[3] = 255;
    }
}

/// 截取屏幕指定区域（物理像素，虚拟桌面坐标系，左上角原点）。
/// GDI BitBlt 不需要任何系统权限；DRM 保护内容会得到黑块（平台限制）。
pub fn capture_screen_region_px(rect_px: [i32; 4]) -> AppResult<OcrImage> {
    let [x, y, width, height] = rect_px;
    if width <= 0 || height <= 0 {
        return Err(AppError::InvalidInput(
            "capture region is empty".to_string(),
        ));
    }

    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err(AppError::Platform("GetDC failed".to_string()));
        }

        let result = blit_and_encode(screen, x, y, width, height);
        ReleaseDC(None, screen);
        result
    }
}

/// 兼容 DC + 位图 → BitBlt → GetDIBits → PNG；用完恢复并释放 GDI 资源。
unsafe fn blit_and_encode(
    screen: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> AppResult<OcrImage> {
    unsafe {
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let previous = SelectObject(memory, bitmap.into());
        let captured = read_pixels(memory, bitmap, screen, x, y, width, height);
        SelectObject(memory, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        captured
    }
}

unsafe fn read_pixels(
    memory: HDC,
    bitmap: HBITMAP,
    screen: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> AppResult<OcrImage> {
    unsafe {
        // CAPTUREBLT：把分层窗口（含 WPF/Chromium 半透明窗）也截进来。
        let rop = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
        if let Err(error) = BitBlt(memory, 0, 0, width, height, Some(screen), x, y, rop) {
            return Err(AppError::Platform(format!("BitBlt failed: {error}")));
        }

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // 负值：自上而下的顶层 DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buffer = vec![0u8; width as usize * height as usize * 4];
        let copied = GetDIBits(
            memory,
            bitmap,
            0,
            height as u32,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            &mut info,
            DIB_RGB_COLORS,
        );
        if copied == 0 {
            return Err(AppError::Platform("GetDIBits failed".to_string()));
        }

        bgra_to_opaque_rgba(&mut buffer);
        let pixels = image::RgbaImage::from_raw(width as u32, height as u32, buffer)
            .ok_or_else(|| AppError::Internal("capture buffer size mismatch".to_string()))?;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|error| AppError::Ocr(format!("failed to encode capture: {error}")))?;
        OcrImage::from_png(png)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn bgra_becomes_opaque_rgba() {
        // GDI 顺序 B,G,R,A → RGBA；alpha 即使是 0 也强制不透明。
        let mut buffer = [0x11u8, 0x22, 0x33, 0x00];
        bgra_to_opaque_rgba(&mut buffer);
        assert_eq!(buffer, [0x33, 0x22, 0x11, 0xFF]);
    }

    #[test]
    fn bgra_conversion_handles_multiple_pixels() {
        let mut buffer = [1u8, 2, 3, 4, 5, 6, 7, 8];
        bgra_to_opaque_rgba(&mut buffer);
        assert_eq!(buffer, [3, 2, 1, 255, 7, 6, 5, 255]);
    }
}
