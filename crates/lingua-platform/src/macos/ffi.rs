//! macOS Accessibility / CoreGraphics 最小 FFI。
//!
//! 本文件是整个 workspace 中唯一允许 `unsafe` 的地方
//! （workspace lints 对其它 crate 一律 deny）。
//!
//! 权限说明：读取其它应用的 AX 属性、以及 `CGEventPost` 模拟按键，
//! 都要求用户在「系统设置 → 隐私与安全性 → 辅助功能」中授权本应用。
//! 未授权时通过 [`AppError::Permission`] 上报，由前端做引导。
#![allow(unsafe_code)]

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use lingua_core::{AppError, AppResult};
use std::ffi::c_void;
use std::ptr;

// MARK: - FFI 声明

// ApplicationServices（HIServices）中的 Accessibility API。
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> *mut c_void;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: CFStringRef,
        value: *mut *mut c_void,
    ) -> i32;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
}

// CoreFoundation 的类型查询与释放。
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFGetTypeID(cf: *const c_void) -> usize;
    fn CFRelease(cf: *const c_void);
}

// CoreGraphics：光标位置与按键模拟。
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    fn CGEventCreateKeyboardEvent(
        source: *const c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *const c_void);
}

#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}

/// `kVK_ANSI_C`
const KEY_CODE_C: u16 = 8;
/// `kCGEventFlagMaskCommand`
const EVENT_FLAG_COMMAND: u64 = 1 << 20;
/// `kCGSessionEventTap`
const SESSION_EVENT_TAP: u32 = 1;

// AXError（部分）
const AX_ERROR_SUCCESS: i32 = 0;
const AX_ERROR_API_DISABLED: i32 = -25211;

/// 把 AXError 翻译成可读名称（仅用于日志）。
fn ax_error_name(status: i32) -> String {
    match status {
        AX_ERROR_SUCCESS => "success".into(),
        -25200 => "failure".into(),
        -25201 => "illegal argument".into(),
        -25202 => "invalid UI element".into(),
        -25204 => "cannot complete".into(),
        -25205 => "attribute unsupported".into(),
        -25208 => "not implemented".into(),
        AX_ERROR_API_DISABLED => "API disabled".into(),
        -25212 => "no value".into(),
        other => format!("AXError {other}"),
    }
}

// MARK: - 安全封装

/// 当前进程是否已被授予辅助功能权限。
pub fn is_accessibility_trusted() -> bool {
    let prompt_key = CFString::new("AXTrustedCheckOptionPrompt");
    let pairs = [(prompt_key, CFBoolean::false_value())];
    let options = CFDictionary::from_CFType_pairs(&pairs);
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef() as *const c_void) != 0 }
}

/// 当前光标的全局坐标（逻辑点，屏幕左上角为原点）。
pub fn cursor_position() -> Option<(f64, f64)> {
    unsafe {
        let event = CGEventCreate(ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x, point.y))
    }
}

/// 向当前前台应用发送一次 Cmd+C（按下 + 抬起）。
/// 需要辅助功能权限；失败只能说明权限缺失，由调用方前置检查。
pub fn post_copy_shortcut() {
    unsafe {
        let key_down = CGEventCreateKeyboardEvent(ptr::null(), KEY_CODE_C, true);
        CGEventSetFlags(key_down, EVENT_FLAG_COMMAND);
        CGEventPost(SESSION_EVENT_TAP, key_down);
        CFRelease(key_down);

        let key_up = CGEventCreateKeyboardEvent(ptr::null(), KEY_CODE_C, false);
        CGEventSetFlags(key_up, EVENT_FLAG_COMMAND);
        CGEventPost(SESSION_EVENT_TAP, key_up);
        CFRelease(key_up);
    }
}

/// 通过 Accessibility API 读取当前选中文本。
///
/// 失败语义（与 SelectionService 的 fallback 约定对应）：
/// * 未授权 → [`AppError::Permission`]（fallback 也没有意义，直接终止）；
/// * API 被禁用 → [`AppError::Permission`]；
/// * 目标应用不支持该属性 / 当前没有选区 / 其它 AX 错误 →
///   [`AppError::NoSelection`]（让上层尝试剪贴板 fallback）。
pub fn capture_selected_text() -> AppResult<String> {
    if !is_accessibility_trusted() {
        return Err(AppError::Permission(
            "accessibility permission is required; grant it in System Settings → \
             Privacy & Security → Accessibility"
                .to_string(),
        ));
    }

    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return Err(AppError::Platform(
                "failed to create system-wide element".into(),
            ));
        }

        let result = (|| -> AppResult<String> {
            let focused = copy_attribute(system_wide, "AXFocusedUIElement")
                .map_err(|status| classify_ax_error(status, "AXFocusedUIElement"))?;
            let selected = copy_attribute(focused, "AXSelectedText")
                .map_err(|status| classify_ax_error(status, "AXSelectedText"));
            CFRelease(focused);
            let value = selected?;

            if CFGetTypeID(value) != CFString::type_id() {
                CFRelease(value);
                tracing::debug!("AXSelectedText is not a string; treating as no selection");
                return Err(AppError::NoSelection);
            }
            let text = CFString::wrap_under_create_rule(value as CFStringRef).to_string();
            Ok(text)
        })();

        CFRelease(system_wide);
        result
    }
}

/// 读取单个 AX 属性；返回的指针带 +1 引用计数，由调用方释放。
unsafe fn copy_attribute(element: *const c_void, attribute: &str) -> Result<*mut c_void, i32> {
    let attribute = CFString::new(attribute);
    let mut value: *mut c_void = ptr::null_mut();
    // edition 2024：unsafe fn 体内的 unsafe 操作需要显式 unsafe 块。
    unsafe {
        let status =
            AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut value);
        if status == AX_ERROR_SUCCESS && !value.is_null() {
            Ok(value)
        } else {
            if !value.is_null() {
                CFRelease(value);
            }
            Err(status)
        }
    }
}

fn classify_ax_error(status: i32, attribute: &str) -> AppError {
    if status == AX_ERROR_API_DISABLED {
        return AppError::Permission("accessibility API is disabled".to_string());
    }
    tracing::debug!(attribute, status = %ax_error_name(status), "AX attribute unavailable");
    AppError::NoSelection
}
