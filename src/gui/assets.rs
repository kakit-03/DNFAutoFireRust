// Loads GUI fonts, icons, assets, and native window handles.

use anyhow::{Context, Result, bail};
use eframe::CreationContext;
use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use image::{ImageReader, imageops::FilterType};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::fs;
use std::path::{Path, PathBuf};
use tray_icon::Icon;
use windows::Win32::Foundation::HWND;

use super::{APP_ICON_SIZE, FONT_CANDIDATES, TRAY_ICON_SIZE};
pub(super) fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    if let Some(bytes) = load_cjk_font() {
        fonts
            .font_data
            .insert("cjk".to_string(), FontData::from_owned(bytes).into());
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.insert(0, "cjk".to_string());
        }
        if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
            family.push("cjk".to_string());
        }
    }
    ctx.set_fonts(fonts);
}

fn load_cjk_font() -> Option<Vec<u8>> {
    let font_dir = std::env::var("WINDIR")
        .ok()
        .map(PathBuf::from)?
        .join("Fonts");
    for candidate in FONT_CANDIDATES {
        let path = font_dir.join(candidate);
        if let Ok(bytes) = fs::read(path) {
            return Some(bytes);
        }
    }
    None
}

pub(super) fn hwnd_from_creation_context(cc: &CreationContext<'_>) -> Result<HWND> {
    let handle = cc.window_handle()?.as_raw();
    match handle {
        RawWindowHandle::Win32(win32) => Ok(HWND(win32.hwnd.get() as _)),
        _ => bail!("unsupported platform window handle"),
    }
}

pub(super) fn project_asset_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

pub(super) fn load_window_icon(path: &Path) -> Result<egui::IconData> {
    let image = ImageReader::open(path)
        .with_context(|| format!("failed to open window icon '{}'", path.display()))?
        .decode()
        .with_context(|| format!("failed to decode window icon '{}'", path.display()))?;
    let rgba = image
        .resize_exact(APP_ICON_SIZE, APP_ICON_SIZE, FilterType::Lanczos3)
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

pub(super) fn load_tray_icon_base(path: &Path) -> Result<image::RgbaImage> {
    let image = ImageReader::open(&path)
        .with_context(|| format!("failed to open tray icon '{}'", path.display()))?
        .decode()
        .with_context(|| format!("failed to decode tray icon '{}'", path.display()))?;
    Ok(image
        .resize_exact(TRAY_ICON_SIZE, TRAY_ICON_SIZE, FilterType::Lanczos3)
        .into_rgba8())
}

pub(super) fn tray_icon_with_status_dot(
    base: &image::RgbaImage,
    dot_color: [u8; 4],
) -> Result<Icon> {
    let mut rgba = base.clone();
    paint_status_dot(&mut rgba, dot_color);
    let (width, height) = rgba.dimensions();
    Icon::from_rgba(rgba.into_raw(), width, height)
        .context("failed to build tray icon with status dot")
}

fn paint_status_dot(image: &mut image::RgbaImage, dot_color: [u8; 4]) {
    let width = image.width() as i32;
    let height = image.height() as i32;
    let outer_radius = 8i32;
    let inner_radius = 7i32;
    let center_x = width - outer_radius;
    let center_y = height - outer_radius;

    for y in (center_y - outer_radius)..=(center_y + outer_radius) {
        if !(0..height).contains(&y) {
            continue;
        }
        for x in (center_x - outer_radius)..=(center_x + outer_radius) {
            if !(0..width).contains(&x) {
                continue;
            }

            let dx = x - center_x;
            let dy = y - center_y;
            let distance_squared = dx * dx + dy * dy;
            if distance_squared > outer_radius * outer_radius {
                continue;
            }

            let pixel = if distance_squared <= inner_radius * inner_radius {
                image::Rgba(dot_color)
            } else {
                image::Rgba([255, 255, 255, 255])
            };
            image.put_pixel(x as u32, y as u32, pixel);
        }
    }
}
