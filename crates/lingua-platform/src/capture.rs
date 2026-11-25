//! 屏幕截图能力。
//!
//! macOS 实现用系统自带 `screencapture` CLI（`-x -R x,y,w,h`）：
//! 不引入截屏 FFI / ScreenCaptureKit 绑定，且天然走系统屏幕录制
//! 权限管控。截图先写临时文件、读入内存后立即删除，**不落盘保留**
//! （需求约束：OCR 原始图片默认不保存）。
//!
//! 需要「屏幕录制」权限；未授权时截图只会得到桌面壁纸而无窗口内容，
//! 所以调用方应先用 [`screen_recording_granted`] 预检。
//!
//! 多显示器：每块屏一张快照（[`capture_monitor`]），连同它在虚拟桌面里的
//! 逻辑矩形组成 [`DesktopCapture`]；选区用虚拟桌面逻辑坐标表达，单屏选区
//! 直接裁剪、跨屏选区拼接成一张图。

use lingua_core::{AppError, AppResult, OcrImage, Rect};

/// 从整屏截图中裁剪出目标区域（像素坐标），输出新 PNG。
/// 阻塞调用，上层用 `spawn_blocking` 包裹。
pub fn crop_image(image: &OcrImage, rect_px: [u32; 4]) -> AppResult<OcrImage> {
    let [x, y, width, height] = rect_px;
    if width == 0 || height == 0 {
        return Err(AppError::InvalidInput("crop region is empty".to_string()));
    }
    let decoded = ::image::load_from_memory(&image.png)
        .map_err(|error| AppError::Ocr(format!("failed to decode capture: {error}")))?;
    let x = x.min(decoded.width().saturating_sub(1));
    let y = y.min(decoded.height().saturating_sub(1));
    let cropped = decoded.crop_imm(
        x,
        y,
        width.min(decoded.width() - x),
        height.min(decoded.height() - y),
    );
    let mut png = Vec::new();
    cropped
        .write_to(
            &mut std::io::Cursor::new(&mut png),
            ::image::ImageFormat::Png,
        )
        .map_err(|error| AppError::Ocr(format!("failed to encode crop: {error}")))?;
    OcrImage::from_png(png)
}

/// 一块显示器的几何信息（tauri `Monitor` 报告的原样数据）。
///
/// 矩形是物理像素，与 `Monitor::position()` / `Monitor::size()` 一致；
/// 逻辑矩形 = 物理矩形 ÷ `scale_factor`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// 物理像素 / 逻辑点。
    pub scale_factor: f64,
}

impl MonitorGeometry {
    /// 虚拟桌面逻辑坐标系里的矩形（macOS 全局点 / Windows 逻辑像素）。
    /// 主显示器左上角为原点，排在左侧 / 上方的显示器坐标为负。
    pub fn logical_rect(&self) -> Rect {
        let scale = self.effective_scale();
        Rect {
            x: f64::from(self.x) / scale,
            y: f64::from(self.y) / scale,
            width: f64::from(self.width) / scale,
            height: f64::from(self.height) / scale,
        }
    }

    /// 物理像素矩形（虚拟桌面坐标系）；只有收物理像素的 Windows GDI 用。
    #[cfg(target_os = "windows")]
    fn physical_rect(&self) -> Rect {
        Rect {
            x: f64::from(self.x),
            y: f64::from(self.y),
            width: f64::from(self.width),
            height: f64::from(self.height),
        }
    }

    /// 缩放比异常（0 / NaN）时按 1 处理，不让坏数据变成除零。
    fn effective_scale(&self) -> f64 {
        if self.scale_factor.is_finite() && self.scale_factor > 0.0 {
            self.scale_factor
        } else {
            1.0
        }
    }
}

/// 一块显示器的快照。
#[derive(Debug, Clone)]
pub struct MonitorCapture {
    pub image: OcrImage,
    /// 该显示器在虚拟桌面逻辑坐标系里的矩形。
    pub rect: Rect,
}

/// 一次多屏截图会话的全部快照。
///
/// 选区一律用虚拟桌面逻辑坐标表达，与「落在哪块屏」无关：单屏选区直接在
/// 该屏快照上裁剪；跨屏选区按逻辑坐标把各屏的相交部分拼到同一张画布
/// （混合 DPI 时重采样到与选区重叠最多那块屏的分辨率）。
#[derive(Debug, Clone)]
pub struct DesktopCapture {
    pub monitors: Vec<MonitorCapture>,
}

impl DesktopCapture {
    pub fn new(monitors: Vec<MonitorCapture>) -> AppResult<Self> {
        if monitors.is_empty() {
            return Err(AppError::InvalidInput(
                "no display was captured".to_string(),
            ));
        }
        Ok(Self { monitors })
    }

    /// 把某块屏上 overlay 的 CSS 像素选区换算成虚拟桌面逻辑坐标。
    ///
    /// `viewport` 是该 overlay 的 CSS 视口尺寸（前端上报，HiDPI 下唯一可信的
    /// CSS 尺寸来源）：CSS 像素 → 逻辑点的比例 = 显示器逻辑尺寸 / 视口，
    /// 窗口铺满显示器时恒为 1。`rect` 可以越出该显示器（跨屏拖拽）。
    pub fn selection_from_viewport(
        &self,
        monitor_index: usize,
        rect: Rect,
        viewport: (f64, f64),
    ) -> AppResult<Rect> {
        let monitor = self.monitors.get(monitor_index).ok_or_else(|| {
            AppError::InvalidInput(format!("no captured display at index {monitor_index}"))
        })?;
        let to_logical_x = logical_per_css(monitor.rect.width, viewport.0);
        let to_logical_y = logical_per_css(monitor.rect.height, viewport.1);
        Ok(Rect {
            x: monitor.rect.x + rect.x * to_logical_x,
            y: monitor.rect.y + rect.y * to_logical_y,
            width: rect.width * to_logical_x,
            height: rect.height * to_logical_y,
        })
    }

    /// 裁剪选区，输出送 OCR 的图像。阻塞调用，上层用 `spawn_blocking` 包裹。
    ///
    /// 选区整体落在一块屏内 = 直接在该屏快照上裁剪（与单屏时代逐字节同路）；
    /// 跨屏 = 以重叠最多的那块屏的分辨率为准，各屏交集按逻辑坐标拼进画布，
    /// 屏幕间隙（非矩形排布的空洞）填黑。
    pub fn crop(&self, selection: &Rect) -> AppResult<OcrImage> {
        if !is_positive(selection) {
            return Err(AppError::InvalidInput("crop region is empty".to_string()));
        }
        let mut hits: Vec<(usize, Rect)> = self
            .monitors
            .iter()
            .enumerate()
            .filter_map(|(index, monitor)| {
                intersect(&monitor.rect, selection).map(|rect| (index, rect))
            })
            .collect();
        if hits.is_empty() {
            return Err(AppError::InvalidInput(
                "selection does not intersect any display".to_string(),
            ));
        }
        // 重叠最多的排首位：它决定跨屏拼接的输出分辨率。
        hits.sort_by(|(_, a), (_, b)| area(b).total_cmp(&area(a)));

        if let [(index, visible)] = hits[..] {
            let monitor = &self.monitors[index];
            let [x, y, width, height] = pixel_rect(
                visible,
                (monitor.rect.x, monitor.rect.y),
                (pixel_scale_x(monitor), pixel_scale_y(monitor)),
                (monitor.image.width, monitor.image.height),
            )?;
            return crop_image(&monitor.image, [x, y, width, height]);
        }

        let primary = &self.monitors[hits[0].0];
        let output_scale = (pixel_scale_x(primary), pixel_scale_y(primary));
        let canvas_side =
            |length: f64, scale: f64| pixel_span(0.0, length, scale, u32::MAX).map(|(_, end)| end);
        let (Some(canvas_width), Some(canvas_height)) = (
            canvas_side(selection.width, output_scale.0),
            canvas_side(selection.height, output_scale.1),
        ) else {
            return Err(AppError::InvalidInput("crop region is empty".to_string()));
        };
        let mut canvas = ::image::RgbaImage::from_pixel(
            canvas_width,
            canvas_height,
            ::image::Rgba([0, 0, 0, 255]),
        );

        for (index, visible) in hits {
            let monitor = &self.monitors[index];
            let decoded = ::image::load_from_memory(&monitor.image.png)
                .map_err(|error| AppError::Ocr(format!("failed to decode capture: {error}")))?
                .to_rgba8();
            let [sx, sy, sw, sh] = pixel_rect(
                visible,
                (monitor.rect.x, monitor.rect.y),
                (pixel_scale_x(monitor), pixel_scale_y(monitor)),
                (monitor.image.width, monitor.image.height),
            )?;
            let [dx, dy, dw, dh] = pixel_rect(
                visible,
                (selection.x, selection.y),
                output_scale,
                (canvas_width, canvas_height),
            )?;
            let mut piece = ::image::imageops::crop_imm(&decoded, sx, sy, sw, sh).to_image();
            if (sw, sh) != (dw, dh) {
                // 混合 DPI：该屏拼块重采样到输出分辨率。
                piece = ::image::imageops::resize(
                    &piece,
                    dw,
                    dh,
                    ::image::imageops::FilterType::Triangle,
                );
            }
            ::image::imageops::overlay(&mut canvas, &piece, i64::from(dx), i64::from(dy));
        }

        let mut png = Vec::new();
        canvas
            .write_to(
                &mut std::io::Cursor::new(&mut png),
                ::image::ImageFormat::Png,
            )
            .map_err(|error| AppError::Ocr(format!("failed to encode crop: {error}")))?;
        OcrImage::from_png(png)
    }
}

/// 矩形尺寸为正且各分量有限。
fn is_positive(rect: &Rect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
        && rect.width >= 1.0
        && rect.height >= 1.0
}

fn area(rect: &Rect) -> f64 {
    rect.width * rect.height
}

/// 两个矩形的交集；不相交为 `None`。
fn intersect(a: &Rect, b: &Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > x && bottom > y).then_some(Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

/// 该显示器快照每个逻辑点对应的像素数（横 / 纵各自计算，截图与显示器
/// 尺寸不成比例时也不错位）。
fn pixel_scale_x(monitor: &MonitorCapture) -> f64 {
    f64::from(monitor.image.width) / monitor.rect.width
}

fn pixel_scale_y(monitor: &MonitorCapture) -> f64 {
    f64::from(monitor.image.height) / monitor.rect.height
}

/// `visible`（虚拟桌面逻辑坐标）相对 `origin` 的偏移，按 `(scale_x, scale_y)`
/// 落到 `limit` 尺寸的像素图上的 `[x, y, width, height]`。
///
/// 裁剪某块屏时 `origin` / `scale` / `limit` 都取该显示器自身；跨屏拼接时
/// 目标位置改以选区为原点、以输出分辨率为比例、以画布为边界。
fn pixel_rect(
    visible: Rect,
    origin: (f64, f64),
    scale: (f64, f64),
    limit: (u32, u32),
) -> AppResult<[u32; 4]> {
    let (Some((x0, x1)), Some((y0, y1))) = (
        pixel_span(visible.x - origin.0, visible.width, scale.0, limit.0),
        pixel_span(visible.y - origin.1, visible.height, scale.1, limit.1),
    ) else {
        return Err(AppError::InvalidInput("crop region is empty".to_string()));
    };
    Ok([x0, y0, x1 - x0, y1 - y0])
}

/// `[offset, offset + length]`（逻辑单位）映射到像素轴上取整后的 `[起, 止]`，
/// 钳制在 `[0, limit]` 内；取整后长度为 0 时返回 `None`。
///
/// 两端各自取整而不是「起点取整 + 长度取整」：相邻显示器各自的交集在接缝处
/// 算出同一个整数边界，跨屏拼块严格相接（不重叠、不留缝）。
fn pixel_span(offset: f64, length: f64, scale: f64, limit: u32) -> Option<(u32, u32)> {
    let limit = f64::from(limit);
    let start = (offset * scale).round().clamp(0.0, limit) as u32;
    let end = ((offset + length) * scale)
        .round()
        .clamp(f64::from(start), limit) as u32;
    (end > start).then_some((start, end))
}

/// CSS 像素 → 逻辑点的比例：视口尺寸非法（缺失 / 0 / NaN）时按 1:1。
fn logical_per_css(logical: f64, viewport: f64) -> f64 {
    if viewport.is_finite() && viewport >= 1.0 {
        logical / viewport
    } else {
        1.0
    }
}

/// 屏幕录制权限是否已授予。
/// Windows 的 GDI 截图不需要任何系统权限，视为已授予；
/// Linux 无实现时返回 `false`。
pub fn screen_recording_granted() -> bool {
    #[cfg(target_os = "macos")]
    return crate::macos::ffi::is_screen_recording_granted();

    #[cfg(target_os = "windows")]
    return true;

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    false
}

/// 请求屏幕录制权限（macOS 会弹系统授权框）。返回是否已授予。
/// Windows / 其它平台无权限概念，直接返回授权结果。
pub fn request_screen_recording_access() -> bool {
    #[cfg(target_os = "macos")]
    return crate::macos::ffi::request_screen_recording_access();

    #[cfg(target_os = "windows")]
    return true;

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    false
}

/// 截取一整块显示器，返回 PNG 图像。阻塞调用，上层用 `spawn_blocking` 包裹。
///
/// 坐标系按平台各自解释：macOS `screencapture -R` 收**全局点**（tauri 在
/// macOS 上报的「物理像素」= 点 × scale，Retina 上必须除回去，否则截到
/// 2 倍大的区域）；Windows GDI 收物理像素。
pub fn capture_monitor(geometry: &MonitorGeometry) -> AppResult<OcrImage> {
    #[cfg(target_os = "macos")]
    return capture_region_macos(&geometry.logical_rect());

    #[cfg(target_os = "windows")]
    return capture_region_windows(&geometry.physical_rect());

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = geometry;
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

/// Windows：GDI BitBlt 截图（物理像素），无需系统权限。
#[cfg(target_os = "windows")]
fn capture_region_windows(rect: &Rect) -> AppResult<OcrImage> {
    if rect.width < 1.0 || rect.height < 1.0 {
        return Err(AppError::InvalidInput(
            "capture region is empty".to_string(),
        ));
    }
    crate::windows::ffi::capture_screen_region_px([
        rect.x.round() as i32,
        rect.y.round() as i32,
        rect.width.round() as i32,
        rect.height.round() as i32,
    ])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// 一块逻辑 `100×100` 的显示器快照；`pixels` 是快照的边长（100 = 1×，200 = 2×）。
    fn display(x: f64, y: f64, pixels: u32, rgba: [u8; 4]) -> MonitorCapture {
        let image = ::image::RgbaImage::from_pixel(pixels, pixels, ::image::Rgba(rgba));
        let mut png = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut png),
                ::image::ImageFormat::Png,
            )
            .unwrap();
        MonitorCapture {
            image: OcrImage::from_png(png).unwrap(),
            rect: rect(x, y, 100.0, 100.0),
        }
    }

    /// 左右并排的两块 1× 屏：左红右蓝。
    fn side_by_side() -> DesktopCapture {
        DesktopCapture::new(vec![
            display(0.0, 0.0, 100, RED),
            display(100.0, 0.0, 100, BLUE),
        ])
        .unwrap()
    }

    fn pixel_of(image: &OcrImage, x: u32, y: u32) -> [u8; 4] {
        ::image::load_from_memory(&image.png)
            .unwrap()
            .to_rgba8()
            .get_pixel(x, y)
            .0
    }

    #[test]
    fn logical_rect_divides_physical_by_scale_factor() {
        // Retina 副屏排在主屏左侧：物理 (-2880, 0, 2880×1800)，2×。
        let geometry = MonitorGeometry {
            x: -2880,
            y: 0,
            width: 2880,
            height: 1800,
            scale_factor: 2.0,
        };
        assert_eq!(geometry.logical_rect(), rect(-1440.0, 0.0, 1440.0, 900.0));
    }

    #[test]
    fn degenerate_scale_factor_falls_back_to_one() {
        let mut geometry = MonitorGeometry {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale_factor: 0.0,
        };
        assert_eq!(geometry.logical_rect(), rect(0.0, 0.0, 1920.0, 1080.0));
        geometry.scale_factor = f64::NAN;
        assert_eq!(geometry.logical_rect(), rect(0.0, 0.0, 1920.0, 1080.0));
    }

    #[test]
    fn empty_capture_is_rejected() {
        assert!(DesktopCapture::new(Vec::new()).is_err());
    }

    #[test]
    fn single_display_selection_crops_that_display() {
        let desktop = side_by_side();
        let cropped = desktop.crop(&rect(120.0, 10.0, 30.0, 40.0)).unwrap();
        assert_eq!((cropped.width, cropped.height), (30, 40));
        assert_eq!(pixel_of(&cropped, 15, 20), BLUE);
    }

    #[test]
    fn cross_display_selection_is_stitched() {
        let desktop = side_by_side();
        // 横跨两屏：左半落在红屏，右半落在蓝屏。
        let cropped = desktop.crop(&rect(50.0, 10.0, 100.0, 20.0)).unwrap();
        assert_eq!((cropped.width, cropped.height), (100, 20));
        assert_eq!(pixel_of(&cropped, 49, 5), RED, "接缝左侧来自左屏");
        assert_eq!(
            pixel_of(&cropped, 50, 5),
            BLUE,
            "接缝右侧来自右屏，无缝隙无重叠"
        );
    }

    #[test]
    fn vertically_stacked_displays_are_stitched() {
        let desktop = DesktopCapture::new(vec![
            display(0.0, 0.0, 100, RED),
            display(0.0, 100.0, 100, GREEN),
        ])
        .unwrap();
        let cropped = desktop.crop(&rect(10.0, 50.0, 20.0, 100.0)).unwrap();
        assert_eq!((cropped.width, cropped.height), (20, 100));
        assert_eq!(pixel_of(&cropped, 10, 25), RED);
        assert_eq!(pixel_of(&cropped, 10, 75), GREEN);
    }

    #[test]
    fn display_left_of_primary_uses_negative_coordinates() {
        let desktop = DesktopCapture::new(vec![
            display(0.0, 0.0, 100, RED),
            display(-100.0, 0.0, 100, BLUE),
        ])
        .unwrap();
        let cropped = desktop.crop(&rect(-60.0, 10.0, 100.0, 20.0)).unwrap();
        assert_eq!((cropped.width, cropped.height), (100, 20));
        assert_eq!(pixel_of(&cropped, 20, 5), BLUE);
        assert_eq!(pixel_of(&cropped, 60, 5), RED);
    }

    #[test]
    fn cross_display_selection_resamples_mixed_dpi() {
        // 左屏 2×（200px 快照 / 100 逻辑点），右屏 1×。
        let desktop = DesktopCapture::new(vec![
            display(0.0, 0.0, 200, RED),
            display(100.0, 0.0, 100, BLUE),
        ])
        .unwrap();

        // 两屏各占一半：面积相同取排在前面的左屏，输出 2×。
        let cropped = desktop.crop(&rect(50.0, 0.0, 100.0, 50.0)).unwrap();
        assert_eq!((cropped.width, cropped.height), (200, 100));
        assert_eq!(pixel_of(&cropped, 50, 50), RED);
        assert_eq!(
            pixel_of(&cropped, 150, 50),
            BLUE,
            "1× 屏的拼块放大到 2× 画布"
        );

        // 右屏占大头：输出跟随右屏的 1×。
        let cropped = desktop.crop(&rect(60.0, 0.0, 100.0, 50.0)).unwrap();
        assert_eq!((cropped.width, cropped.height), (100, 50));
        assert_eq!(pixel_of(&cropped, 20, 25), RED, "2× 屏的拼块缩到 1× 画布");
        assert_eq!(pixel_of(&cropped, 50, 25), BLUE);
    }

    #[test]
    fn selection_inside_gap_is_rejected() {
        let desktop = DesktopCapture::new(vec![
            display(0.0, 0.0, 100, RED),
            display(200.0, 0.0, 100, BLUE),
        ])
        .unwrap();
        let error = desktop.crop(&rect(110.0, 10.0, 50.0, 20.0)).unwrap_err();
        assert!(matches!(error, AppError::InvalidInput(_)), "{error:?}");
    }

    #[test]
    fn selection_partly_inside_gap_crops_the_visible_part() {
        let desktop = DesktopCapture::new(vec![
            display(0.0, 0.0, 100, RED),
            display(200.0, 0.0, 100, BLUE),
        ])
        .unwrap();
        let cropped = desktop.crop(&rect(80.0, 10.0, 60.0, 20.0)).unwrap();
        assert_eq!((cropped.width, cropped.height), (20, 20));
        assert_eq!(pixel_of(&cropped, 10, 10), RED);
    }

    #[test]
    fn empty_or_non_finite_selection_is_rejected() {
        let desktop = side_by_side();
        assert!(desktop.crop(&rect(10.0, 10.0, 0.0, 20.0)).is_err());
        assert!(desktop.crop(&rect(f64::NAN, 10.0, 20.0, 20.0)).is_err());
    }

    #[test]
    fn selection_from_viewport_adds_display_origin() {
        // 副屏排在主屏左侧：同样的 CSS 坐标落在负的全局坐标上。
        let desktop = DesktopCapture::new(vec![
            display(-100.0, 0.0, 100, BLUE),
            display(0.0, 0.0, 100, RED),
        ])
        .unwrap();
        // overlay 铺满显示器：CSS 像素 == 逻辑点，只平移原点。
        let selection = desktop
            .selection_from_viewport(0, rect(10.0, 20.0, 30.0, 40.0), (100.0, 100.0))
            .unwrap();
        assert_eq!(selection, rect(-90.0, 20.0, 30.0, 40.0));
        // 越出本屏的拖拽照常换算（负宽由前端归一化，这里只管平移缩放）。
        let selection = desktop
            .selection_from_viewport(1, rect(90.0, 0.0, 50.0, 10.0), (100.0, 100.0))
            .unwrap();
        assert_eq!(selection, rect(90.0, 0.0, 50.0, 10.0));
    }

    #[test]
    fn selection_from_viewport_scales_by_viewport_ratio() {
        let desktop = side_by_side();
        // 视口只有显示器逻辑宽度的一半：1 CSS 像素 = 2 逻辑点。
        let selection = desktop
            .selection_from_viewport(0, rect(10.0, 20.0, 30.0, 40.0), (50.0, 100.0))
            .unwrap();
        assert_eq!(selection, rect(20.0, 20.0, 60.0, 40.0));
    }

    #[test]
    fn selection_from_viewport_falls_back_without_viewport() {
        let desktop = side_by_side();
        let selection = desktop
            .selection_from_viewport(1, rect(10.0, 20.0, 30.0, 40.0), (0.0, f64::NAN))
            .unwrap();
        assert_eq!(selection, rect(110.0, 20.0, 30.0, 40.0));
    }

    #[test]
    fn selection_from_unknown_display_is_rejected() {
        let desktop = side_by_side();
        let error = desktop
            .selection_from_viewport(7, rect(0.0, 0.0, 10.0, 10.0), (100.0, 100.0))
            .unwrap_err();
        assert!(matches!(error, AppError::InvalidInput(_)), "{error:?}");
    }

    #[test]
    fn pixel_span_rounds_clamps_and_drops_empty_spans() {
        // [0.5, 10.75] × 2 → [1, 22)，被 limit 钳到 20。
        assert_eq!(pixel_span(0.5, 10.25, 2.0, 20), Some((1, 20)));
        // 负偏移钳到 0。
        assert_eq!(pixel_span(-3.0, 5.0, 1.0, 20), Some((0, 2)));
        // 取整后长度为 0。
        assert_eq!(pixel_span(1.1, 0.2, 1.0, 20), None);
        // 相邻区间共享同一边界：[0.5, 2.5) 与 [2.5, 4.5) → (1, 5) 与 (5, 9)。
        assert_eq!(pixel_span(0.5, 2.0, 2.0, 100), Some((1, 5)));
        assert_eq!(pixel_span(2.5, 2.0, 2.0, 100), Some((5, 9)));
    }
}
