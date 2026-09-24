//! 屏幕截图能力。
//!
//! macOS 实现用系统自带 `screencapture` CLI（`-x -R x,y,w,h`）：
//! 不引入截屏 FFI / ScreenCaptureKit 绑定，且天然走系统屏幕录制
//! 权限管控。截图先写临时文件、读入内存后立即删除，**不落盘保留**
//! （需求约束：OCR 原始图片默认不保存）。
//!
//! 需要「屏幕录制」权限；未授权时截图只会得到桌面壁纸而无窗口内容，
//! 所以调用方应先用 [`screen_recording_granted`] 预检。

use lingua_core::{AppError, AppResult, OcrImage, Rect};

/// 屏幕录制权限是否已授予。
pub fn screen_recording_granted() -> bool {
    #[cfg(target_os = "macos")]
    return crate::macos::ffi::is_screen_recording_granted();

    #[cfg(not(target_os = "macos"))]
    false
}

/// 请求屏幕录制权限（macOS 会弹系统授权框）。返回是否已授予。
pub fn request_screen_recording_access() -> bool {
    #[cfg(target_os = "macos")]
    return crate::macos::ffi::request_screen_recording_access();

    #[cfg(not(target_os = "macos"))]
    false
}

/// 截取屏幕指定区域（全局逻辑坐标，左上角原点），返回 PNG 图像。
/// 阻塞调用（内部起子进程），上层用 `spawn_blocking` 包裹。
pub fn capture_region(rect: &Rect) -> AppResult<OcrImage> {
    #[cfg(target_os = "macos")]
    return capture_region_macos(rect);

    #[cfg(not(target_os = "macos"))]
    {
        let _ = rect;
        Err(AppError::Platform(
            "screen capture is not implemented on this platform yet (planned for Phase 10)"
                .to_string(),
        ))
    }
}

#[cfg(target_os = "macos")]
fn capture_region_macos(rect: &Rect) -> AppResult<OcrImage> {
    if rect.width < 1.0 || rect.height < 1.0 {
        return Err(AppError::InvalidInput(
            "capture region is empty".to_string(),
        ));
    }
    let region = format!(
        "{},{},{},{}",
        rect.x.round() as i32,
        rect.y.round() as i32,
        rect.width.round() as i32,
        rect.height.round() as i32
    );
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join(format!(
        "lingua-capture-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos())
            .unwrap_or(0)
    ));

    let output = std::process::Command::new("screencapture")
        .args(["-x", "-R", &region, path.to_string_lossy().as_ref()])
        .output()
        .map_err(|error| AppError::Platform(format!("failed to run screencapture: {error}")))?;

    let result = if !output.status.success() {
        Err(AppError::Platform(format!(
            "screencapture failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    } else {
        match std::fs::read(&path) {
            Ok(bytes) => OcrImage::from_png(bytes),
            Err(error) => Err(AppError::Ocr(format!("failed to read capture: {error}"))),
        }
    };

    let _ = std::fs::remove_file(&path);
    result
}
