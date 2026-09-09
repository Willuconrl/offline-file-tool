//! 水印功能：为图片 / PDF 批量添加多层文字或图片水印（纯本地处理）
//! - 文字水印：系统字体（黑体/雅黑，支持中文）栅格化，支持旋转、九宫格定位、平铺
//! - PDF 水印：每层渲染为图像 XObject + SMask（保持透明度）嵌入每个页面
//! - 支持任意多个水印图层叠加（文字 + 图片混合）

use crate::images::{encode_image, is_heic_path, open_image, unique_path, JobProgress};
use ab_glyph::{Font, ScaleFont};
use base64::Engine;
use image::{DynamicImage, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct WatermarkLayer {
    /// "text" | "image"
    pub kind: String,
    pub text: String,
    /// 文字字号（像素，栅格化分辨率）
    pub font_size: u32,
    /// "#RRGGBB"
    pub color: String,
    /// 0-100
    pub opacity: u8,
    /// 0-360
    pub rotation: i32,
    /// tl | tr | bl | br | center | top | bottom | left | right | tile
    pub position: String,
    pub margin: u32,
    pub tile_gap: u32,
    pub image_path: Option<String>,
    /// 水印宽度占比（图片：占原图宽度 %；PDF：占页面宽度 %）
    pub image_scale: u32,
    /// 每页/每图水印个数（1 为单个定位；>1 时按网格均匀分布，忽略 position）
    pub count: u32,
}

impl Default for WatermarkLayer {
    fn default() -> Self {
        Self {
            kind: "text".into(),
            text: String::new(),
            font_size: 48,
            color: "#808080".into(),
            opacity: 40,
            rotation: 0,
            position: "center".into(),
            margin: 24,
            tile_gap: 60,
            image_path: None,
            image_scale: 40,
            count: 1,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkResult {
    pub path: String,
    pub name: String,
    pub out_path: Option<String>,
    pub out_name: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
}

// ---------- 字体与文字栅格化 ----------

fn parse_color(s: &str) -> [u8; 3] {
    let hex = s.trim_start_matches('#');
    u32::from_str_radix(hex, 16)
        .map(|v| [(v >> 16) as u8, (v >> 8) as u8, v as u8])
        .unwrap_or([128, 128, 128])
}

/// 优先黑体（简体中文系统均内置），其次微软雅黑，最后 Arial
fn load_font() -> Option<(Vec<u8>, u32)> {
    let windir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    let candidates = [
        (windir.join("Fonts\\simhei.ttf"), 0u32),
        (windir.join("Fonts\\msyh.ttc"), 0u32),
        (windir.join("Fonts\\arial.ttf"), 0u32),
    ];
    for (p, idx) in candidates {
        if let Ok(data) = std::fs::read(&p) {
            if ab_glyph::FontRef::try_from_slice_and_index(&data, idx).is_ok() {
                return Some((data, idx));
            }
        }
    }
    None
}

fn render_text_rgba(layer: &WatermarkLayer) -> Result<RgbaImage, String> {
    let text = layer.text.trim();
    if text.is_empty() {
        return Err("@err_wm_empty".to_string());
    }
    let (font_data, index) = load_font().ok_or("@err_wm_no_font")?;
    let font = ab_glyph::FontRef::try_from_slice_and_index(&font_data, index)
        .map_err(|e| format!("@err_wm_no_font|{e}"))?;
    let size = layer.font_size.clamp(8, 512) as f32;
    let scale = ab_glyph::PxScale::from(size);
    let scaled = font.as_scaled(size);

    // 排版：计算总宽度与上下边界
    let mut glyphs: Vec<(f32, ab_glyph::OutlinedGlyph)> = Vec::new();
    let mut cursor = 0f32;
    let mut ascent = 0f32;
    let mut descent = 0f32;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        let g = gid.with_scale(scale);
        if let Some(outlined) = font.outline_glyph(g) {
            let b = outlined.px_bounds();
            ascent = ascent.max(-b.min.y);
            descent = descent.max(b.max.y);
            glyphs.push((cursor, outlined));
        }
        cursor += scaled.h_advance(gid);
    }
    if glyphs.is_empty() {
        return Err("@err_wm_empty".to_string());
    }
    let pad = 4u32;
    let w = (cursor.ceil().max(1.0) as u32) + pad * 2;
    let h = ((ascent + descent).ceil().max(1.0) as u32) + pad * 2;
    let [cr, cg, cb] = parse_color(&layer.color);
    let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];

    for (x0, glyph) in glyphs {
        // draw 回调坐标以字形包围盒为原点，必须加上 px_bounds 偏移
        let b = glyph.px_bounds();
        glyph.draw(|gx, gy, cov| {
            let px = (x0 + b.min.x + pad as f32 + gx as f32).floor() as i64;
            let py = (pad as f32 + ascent + b.min.y + gy as f32).floor() as i64;
            if px < 0 || py < 0 || px >= w as i64 || py >= h as i64 {
                return;
            }
            let i = ((py as usize) * (w as usize) + px as usize) * 4;
            // ab_glyph 的 coverage 为 0.0~1.0（此处不叠加 opacity，统一在输出/覆盖时应用）
            let na = (cov * 255.0).round() as u8;
            if na == 0 {
                return;
            }
            let da = buf[i + 3] as u32;
            // alpha over 合成
            let oa = na as u32 + da * (255 - na as u32) / 255;
            if oa == 0 {
                return;
            }
            let src_w = (na as u32) * 255;
            let dst_w = da * (255 - na as u32);
            let blend = |s: u8, d: u8| -> u8 {
                ((s as u32 * src_w + d as u32 * dst_w) / (255 * oa)).min(255) as u8
            };
            buf[i] = blend(cr, buf[i]);
            buf[i + 1] = blend(cg, buf[i + 1]);
            buf[i + 2] = blend(cb, buf[i + 2]);
            buf[i + 3] = oa as u8;
        });
    }
    RgbaImage::from_raw(w, h, buf).ok_or_else(|| "@err_wm_empty".to_string())
}

/// 双线性插值的任意角度旋转（带透明通道）
fn rotate_rgba(img: &RgbaImage, deg: i32) -> RgbaImage {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let rad = (deg as f64).to_radians();
    let (cos, sin) = (rad.cos(), rad.sin());
    let nw = (w * cos.abs() as f32 + h * sin.abs() as f32).ceil().max(1.0) as u32;
    let nh = (w * sin.abs() as f32 + h * cos.abs() as f32).ceil().max(1.0) as u32;
    let mut out = RgbaImage::new(nw, nh);
    let cx = w / 2.0;
    let cy = h / 2.0;
    let ncx = nw as f32 / 2.0;
    let ncy = nh as f32 / 2.0;
    for y in 0..nh {
        for x in 0..nw {
            let dx = x as f32 + 0.5 - ncx;
            let dy = y as f32 + 0.5 - ncy;
            let sx = dx * cos as f32 + dy * sin as f32 + cx - 0.5;
            let sy = -dx * sin as f32 + dy * cos as f32 + cy - 0.5;
            if sx < -1.0 || sy < -1.0 || sx > w || sy > h {
                continue;
            }
            let x0 = sx.floor() as i64;
            let y0 = sy.floor() as i64;
            let fx = (sx - x0 as f32).clamp(0.0, 1.0);
            let fy = (sy - y0 as f32).clamp(0.0, 1.0);
            let mut acc = [0f32; 4];
            for (oy, wy) in [(0i64, 1.0 - fy), (1i64, fy)] {
                for (ox, wx) in [(0i64, 1.0 - fx), (1i64, fx)] {
                    let px = x0 + ox;
                    let py = y0 + oy;
                    if px >= 0 && py >= 0 && (px as u32) < img.width() && (py as u32) < img.height() {
                        let p = img.get_pixel(px as u32, py as u32).0;
                        let wgt = wx * wy;
                        for c in 0..4 {
                            acc[c] += p[c] as f32 * wgt;
                        }
                    }
                }
            }
            let p = out.get_pixel_mut(x, y);
            for (c, v) in p.0.iter_mut().enumerate() {
                *v = acc[c].round() as u8;
            }
        }
    }
    out
}

/// 生成单个图层的水印位图（文字或图片 + 旋转；不含不透明度）
fn render_layer(layer: &WatermarkLayer) -> Result<RgbaImage, String> {
    let base = match layer.kind.as_str() {
        "image" => {
            let p = layer
                .image_path
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| "@err_wm_img_read|empty".to_string())?;
            let img = image::open(p).map_err(|e| format!("@err_wm_img_read|{e}"))?;
            img.to_rgba8()
        }
        _ => render_text_rgba(layer)?,
    };
    if base.width() == 0 || base.height() == 0 {
        return Err("@err_wm_empty".to_string());
    }
    let rot = layer.rotation.rem_euclid(360);
    if rot == 0 {
        Ok(base)
    } else {
        Ok(rotate_rgba(&base, rot))
    }
}

/// 对位图 alpha 通道统一应用不透明度
fn apply_opacity(img: &mut RgbaImage, opacity: u8) {
    let f = (opacity.clamp(0, 100) as f32) / 100.0;
    for p in img.pixels_mut() {
        p.0[3] = (p.0[3] as f32 * f).round() as u8;
    }
}

/// 把多层水印合成到一张预览画布上（层间 alpha-over 合成）
fn composite_layers(layers: &[WatermarkLayer]) -> Result<RgbaImage, String> {
    if layers.is_empty() {
        return Err("@err_wm_empty".to_string());
    }
    // 画布大小取各层位图的最大尺寸 + 边距
    let mut max_w = 1u32;
    let mut max_h = 1u32;
    let mut rendered: Vec<RgbaImage> = Vec::with_capacity(layers.len());
    for l in layers {
        let mut img = render_layer(l)?;
        apply_opacity(&mut img, l.opacity);
        max_w = max_w.max(img.width() + 16);
        max_h = max_h.max(img.height() + 16);
        rendered.push(img);
    }
    let mut canvas = RgbaImage::from_pixel(max_w, max_h, Rgba([255, 255, 255, 0]));
    for img in &rendered {
        let x = (max_w - img.width()) / 2;
        let y = (max_h - img.height()) / 2;
        for (px, py, p) in img.enumerate_pixels() {
            let src = p.0;
            let a = src[3] as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let d = canvas.get_pixel_mut(x + px, y + py);
            for c in 0..3 {
                let v = src[c] as f32 * a + d.0[c] as f32 * (1.0 - a);
                d.0[c] = v.round() as u8;
            }
            let da = d.0[3] as f32 / 255.0;
            let oa = a + da * (1.0 - a);
            d.0[3] = (oa * 255.0).round() as u8;
        }
    }
    Ok(canvas)
}

// ---------- 定位 ----------

fn single_placement(pos: &str, pw: f32, ph: f32, w: f32, h: f32, m: f32) -> (f32, f32) {
    // 返回左下角坐标（PDF 坐标系，y 向上）
    match pos {
        "tl" => (m, ph - h - m),
        "tr" => (pw - w - m, ph - h - m),
        "bl" => (m, m),
        "br" => (pw - w - m, m),
        "top" => ((pw - w) / 2.0, ph - h - m),
        "bottom" => ((pw - w) / 2.0, m),
        "left" => (m, (ph - h) / 2.0),
        "right" => (pw - w - m, (ph - h) / 2.0),
        _ => ((pw - w) / 2.0, (ph - h) / 2.0),
    }
}

fn placements(
    pos: &str,
    pw: f32,
    ph: f32,
    w: f32,
    h: f32,
    m: f32,
    gap: f32,
    count: u32,
) -> Vec<(f32, f32)> {
    if pos == "tile" {
        let mut out = Vec::new();
        let step_x = w + gap.max(0.0);
        let step_y = h + gap.max(0.0);
        let mut y = m;
        let mut n = 0;
        while y + h <= ph - m && n < 600 {
            let mut x = m;
            while x + w <= pw - m && n < 600 {
                out.push((x, y));
                x += step_x;
                n += 1;
            }
            y += step_y;
        }
        out
    } else if count > 1 {
        grid_placements(count, pw, ph, w, h, m)
    } else {
        vec![single_placement(pos, pw, ph, w, h, m)]
    }
}

/// 按网格均匀分布 n 个水印（PDF 坐标系，返回左下角坐标）。
/// 列数按页面宽高比近似开方，尽量填满矩形。
fn grid_placements(n: u32, pw: f32, ph: f32, w: f32, h: f32, m: f32) -> Vec<(f32, f32)> {
    let n = (n.clamp(1, 100)) as usize;
    let inner_w = (pw - 2.0 * m).max(1.0);
    let inner_h = (ph - 2.0 * m).max(1.0);
    let aspect = inner_w / inner_h;
    let cols = ((n as f32 * aspect).sqrt().ceil() as usize).clamp(1, n);
    let rows = (n + cols - 1) / cols;
    let cell_w = inner_w / cols as f32;
    let cell_h = inner_h / rows as f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let r = i / cols;
        let c = i % cols;
        let x = m + c as f32 * cell_w + (cell_w - w).max(0.0) / 2.0;
        let y = ph - m - h - r as f32 * cell_h - (cell_h - h).max(0.0) / 2.0;
        out.push((x, y));
    }
    out
}

fn overlay_rgba(dst: &mut RgbaImage, wm: &RgbaImage, at: (u32, u32), opacity: f32) {
    for (x, y, p) in wm.enumerate_pixels() {
        let dx = at.0.saturating_add(x);
        let dy = at.1.saturating_add(y);
        if dx >= dst.width() || dy >= dst.height() {
            continue;
        }
        let src = p.0;
        let a = (src[3] as f32 / 255.0) * opacity;
        if a <= 0.0 {
            continue;
        }
        let d = dst.get_pixel_mut(dx, dy);
        for (c, dv) in d.0.iter_mut().take(3).enumerate() {
            let v = src[c] as f32 * a + *dv as f32 * (1.0 - a);
            *dv = v.round() as u8;
        }
    }
}

fn scale_wm(wm: &RgbaImage, target_w: u32) -> RgbaImage {
    let target_w = target_w.max(1);
    if target_w == wm.width() {
        return wm.clone();
    }
    let ratio = target_w as f32 / wm.width() as f32;
    let th = (wm.height() as f32 * ratio).round().max(1.0) as u32;
    let di = DynamicImage::ImageRgba8(wm.clone());
    image::imageops::resize(&di, target_w, th, image::imageops::FilterType::Triangle)
}

// ---------- 图片水印（多层） ----------

fn photo_watermark_one(
    path: &str,
    layers: &[WatermarkLayer],
    out_dir: Option<&Path>,
    used: &mut HashSet<String>,
    inputs: &HashSet<String>,
) -> WatermarkResult {
    let src = Path::new(path);
    let name = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let fail = |e: String| WatermarkResult {
        path: path.to_string(),
        name: name.clone(),
        out_path: None,
        out_name: None,
        ok: false,
        error: Some(e),
    };

    if !is_heic_path(src) {
        if let Err(e) = crate::images::check_dims(src) {
            return fail(e);
        }
    }
    let img = match open_image(src) {
        Ok(i) => i,
        Err(e) => return fail(e),
    };
    let (iw, ih) = (img.width(), img.height());
    let mut rgba = img.to_rgba8();

    for layer in layers {
        let wm = match render_layer(layer) {
            Ok(w) => w,
            Err(e) => return fail(e),
        };
        let target_w = if layer.kind == "image" {
            ((iw as f32) * (layer.image_scale.clamp(5, 100) as f32) / 100.0)
                .round()
                .max(1.0) as u32
        } else {
            wm.width()
        };
        let wm_scaled = scale_wm(&wm, target_w);
        let (ww, wh) = (wm_scaled.width(), wm_scaled.height());
        let opacity = (layer.opacity.clamp(0, 100) as f32) / 100.0;
        let gap = (layer.tile_gap as f32) * (target_w as f32 / wm.width().max(1) as f32);
        for (x, y) in placements(
            &layer.position,
            iw as f32,
            ih as f32,
            ww as f32,
            wh as f32,
            layer.margin as f32,
            gap,
            layer.count,
        ) {
            let py = ih as f32 - y - wh as f32; // 照片像素系 y 向下
            overlay_rgba(
                &mut rgba,
                &wm_scaled,
                (x.round() as u32, py.round() as u32),
                opacity,
            );
        }
    }

    let ext_in = src
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "png".into());
    let heic_in = matches!(ext_in.as_str(), "heic" | "heif");
    let (fmt, ext_out) = if heic_in {
        ("jpeg", "jpg")
    } else {
        (ext_in.as_str(), ext_in.as_str())
    };
    let bytes = match encode_image(&DynamicImage::ImageRgba8(rgba), fmt, 92) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };

    let dir: PathBuf = match out_dir {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => src
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".into());
    let mut out_path = dir.join(format!("{stem}_watermarked.{ext_out}"));
    // 永远不覆盖：批内去重 + 磁盘去重 + 不覆盖批内输入
    let key = out_path.to_string_lossy().to_lowercase();
    if used.contains(&key) || inputs.contains(&key) || out_path.exists() {
        out_path = unique_path(&out_path, used);
    } else {
        used.insert(key);
    }
    if let Err(e) = std::fs::write(&out_path, &bytes) {
        return fail(format!("@err_write|{e}"));
    }
    WatermarkResult {
        path: path.to_string(),
        name,
        out_path: Some(out_path.to_string_lossy().into_owned()),
        out_name: out_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned()),
        ok: true,
        error: None,
    }
}

// ---------- 命令 ----------

/// 水印样式预览：把多层水印合成后返回 PNG（base64 data URL）
#[tauri::command]
pub async fn preview_watermark(layers: Vec<WatermarkLayer>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let canvas = composite_layers(&layers)?;
        let mut buf = Vec::new();
        let mut cur = std::io::Cursor::new(&mut buf);
        DynamicImage::ImageRgba8(canvas)
            .write_to(&mut cur, image::ImageFormat::Png)
            .map_err(|e| format!("@err_encode_png|{e}"))?;
        Ok(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&buf)
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 批量给图片加水印（多层）
#[tauri::command]
pub async fn process_photo_watermarks(
    app: AppHandle,
    job_id: String,
    paths: Vec<String>,
    layers: Vec<WatermarkLayer>,
    out_dir: Option<String>,
) -> Result<Vec<WatermarkResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // 校验图层（提前失败，避免逐文件重复报错）
        for l in &layers {
            render_layer(l)?;
        }
        let total = paths.len() as u32;
        let dir = out_dir.map(PathBuf::from);
        if let Some(d) = dir.as_ref().filter(|d| !d.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(d);
        }
        let inputs: HashSet<String> = paths.iter().map(|p| p.to_lowercase()).collect();
        let mut used = HashSet::new();
        let mut results = Vec::with_capacity(paths.len());
        for (i, p) in paths.iter().enumerate() {
            let _ = app.emit(
                "job",
                JobProgress {
                    job_id: job_id.clone(),
                    op: "watermark".into(),
                    current: (i + 1) as u32,
                    total,
                    message: Some(p.clone()),
                    done: false,
                },
            );
            results.push(photo_watermark_one(p, &layers, dir.as_deref(), &mut used, &inputs));
        }
        let _ = app.emit(
            "job",
            JobProgress {
                job_id,
                op: "watermark".into(),
                current: total,
                total,
                message: None,
                done: true,
            },
        );
        Ok(results)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_layer(text: &str) -> WatermarkLayer {
        let mut l = WatermarkLayer::default();
        l.text = text.into();
        l
    }

    #[test]
    fn text_watermark_renders() {
        let o = sample_layer("测试水印 Test");
        let img = render_text_rgba(&o).unwrap();
        assert!(img.width() > 10 && img.height() > 10);
        let has_alpha = img.pixels().any(|p| p.0[3] > 0);
        assert!(has_alpha, "文字水印应有非透明像素");
        // 字形顶部区域必须有像素（曾因 draw 坐标偏移导致字形整体下移、顶部留空）
        let top_has_pixels = img
            .enumerate_pixels()
            .any(|(_, y, p)| y < img.height() / 3 && p.0[3] > 0);
        assert!(top_has_pixels, "字形应完整落在画布内（顶部 1/3 应有像素）");
        let mut full = render_text_rgba(&o).unwrap();
        let before = full.pixels().map(|p| p.0[3] as u32).sum::<u32>();
        apply_opacity(&mut full, 30);
        let after = full.pixels().map(|p| p.0[3] as u32).sum::<u32>();
        assert!(after < before, "apply_opacity 应降低 alpha");
    }

    #[test]
    fn rotation_produces_valid_image() {
        let o = sample_layer("旋转");
        let img = render_layer(&o).unwrap();
        assert!(img.width() > 10 && img.height() > 10);
        assert!(img.pixels().any(|p| p.0[3] > 0), "旋转后仍有非透明像素");
    }

    #[test]
    fn composite_multiple_layers() {
        let l1 = sample_layer("第一层");
        let l2 = sample_layer("第二层");
        let canvas = composite_layers(&[l1, l2]).unwrap();
        assert!(canvas.pixels().any(|p| p.0[3] > 0));
    }

    #[test]
    fn grid_placements_distributes_evenly() {
        // 10 个水印在 1000×800 页面上 → 4 列 × 3 行，全部落在边界内
        let pts = grid_placements(10, 1000.0, 800.0, 50.0, 30.0, 20.0);
        assert_eq!(pts.len(), 10);
        for &(x, y) in &pts {
            assert!(x >= 19.0 && x + 50.0 <= 981.0, "x 越界: {x}");
            assert!(y >= 19.0 && y + 30.0 <= 781.0, "y 越界: {y}");
        }
        // 同列不同行：第 0 与第 4 个水印 x 相同、y 递减（行从上到下）
        assert!((pts[0].0 - pts[4].0).abs() < 1.0);
        assert!(pts[0].1 > pts[4].1);
        // 数量正确：20 个 → 20 个坐标
        assert_eq!(grid_placements(20, 1000.0, 800.0, 50.0, 30.0, 20.0).len(), 20);
        // placements 入口：count=1 走单点，count=10 走网格
        assert_eq!(placements("center", 1000.0, 800.0, 50.0, 30.0, 20.0, 0.0, 1).len(), 1);
        assert_eq!(placements("center", 1000.0, 800.0, 50.0, 30.0, 20.0, 0.0, 10).len(), 10);
    }

    #[test]
    fn photo_overlay_changes_pixels() {
        let mut o = sample_layer("WM");
        o.opacity = 100;
        o.position = "center".into();
        o.margin = 0;
        let wm = render_layer(&o).unwrap();
        let dir = PathBuf::from("target").join("testdata");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("wm_src.png");
        let base = RgbaImage::from_pixel(200, 120, image::Rgba([255, 255, 255, 255]));
        base.save(&src).unwrap();
        let mut used = HashSet::new();
        let inputs: HashSet<String> = [src.to_string_lossy().to_lowercase()].into_iter().collect();
        let r = photo_watermark_one(&src.to_string_lossy(), &[o], Some(&dir), &mut used, &inputs);
        assert!(r.ok, "err: {:?}", r.error);
        let out = image::open(r.out_path.unwrap()).unwrap().to_rgb8();
        assert!(out.pixels().any(|p| p.0[0] < 250), "水印应改变像素");
    }

}
