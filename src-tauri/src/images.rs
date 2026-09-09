//! 图片压缩与格式转换命令（image crate，纯本地处理）

use base64::Engine;
use image::imageops::FilterType;
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMeta {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    /// 扩展名（大写），如 JPG / PNG / WEBP
    pub format: String,
    /// 色彩类型描述
    pub color: String,
    /// base64 JPEG 缩略图（data URL）
    pub thumb: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageOptions {
    /// keep | jpeg | png | webp
    pub out_format: String,
    /// 1-100，JPEG/WebP 质量
    pub quality: u8,
    pub resize: bool,
    /// percent | max_side
    pub resize_mode: String,
    pub percent: u32,
    pub max_side: u32,
    pub overwrite: bool,
}

impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            out_format: "keep".into(),
            quality: 82,
            resize: false,
            resize_mode: "percent".into(),
            percent: 80,
            max_side: 1920,
            overwrite: false,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageResult {
    pub path: String,
    pub name: String,
    pub out_path: Option<String>,
    pub out_name: Option<String>,
    pub in_size: u64,
    pub out_size: Option<u64>,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job_id: String,
    pub op: String,
    pub current: u32,
    pub total: u32,
    pub message: Option<String>,
    pub done: bool,
}

/// 单张图片最大像素数（1 亿），防止超大图全量解码导致内存耗尽
const MAX_PIXELS: u64 = 100_000_000;

/// 先只读文件头获取尺寸并做上限检查，再全量解码
pub(crate) fn check_dims(path: &Path) -> Result<(u32, u32), String> {
    let reader = image::ImageReader::open(path).map_err(|e| format!("@err_img_read|{e}"))?;
    let dims = reader
        .with_guessed_format()
        .map_err(|e| format!("@err_img_read|{e}"))?
        .into_dimensions()
        .map_err(|e| format!("@err_img_read|{e}"))?;
    if (dims.0 as u64) * (dims.1 as u64) > MAX_PIXELS {
        return Err(format!("@err_img_too_large|{}|{}", dims.0, dims.1));
    }
    Ok(dims)
}

pub(crate) fn is_heic_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("heic") | Some("heif")
    )
}

/// HEIC/HEIF 解码（苹果照片格式，libheif + libde265，纯本地）
fn decode_heic(path: &Path) -> Result<DynamicImage, String> {
    let lib = libheif_rs::LibHeif::new();
    let ctx = libheif_rs::HeifContext::read_from_file(&path.to_string_lossy())
        .map_err(|e| format!("@err_img_read|{e}"))?;
    let handle = ctx
        .primary_image_handle()
        .map_err(|e| format!("@err_img_read|{e}"))?;
    // 10-bit iPhone HEIC 统一转为 8-bit
    let mut opts = libheif_rs::DecodingOptions::new()
        .ok_or_else(|| "@err_img_read|no decoding options".to_string())?;
    opts.set_convert_hdr_to_8bit(true);
    let image = lib
        .decode(
            &handle,
            libheif_rs::ColorSpace::Rgb(libheif_rs::RgbChroma::Rgb),
            Some(opts),
        )
        .map_err(|e| format!("@err_img_read|{e}"))?;
    let w = image.width();
    let h = image.height();
    if w == 0 || h == 0 {
        return Err("@err_img_read|empty image".to_string());
    }
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(format!("@err_img_too_large|{w}|{h}"));
    }
    let planes = image.planes();
    let p = planes
        .interleaved
        .ok_or_else(|| "@err_img_read|no interleaved data".to_string())?;
    let row_bytes = (w as usize) * 3;
    let bytes_per_px = if (h as usize) > 0 {
        p.data.len() / (w as usize * h as usize)
    } else {
        0
    };
    let rgb: Vec<u8> = if bytes_per_px >= 6 {
        // 16 位小端（R G B 各 2 字节）：取每通道高字节
        p.data.iter().skip(1).step_by(2).cloned().collect()
    } else if p.stride == row_bytes {
        p.data.to_vec()
    } else {
        // stride 含对齐填充：逐行拷贝有效像素
        let mut buf = Vec::with_capacity(row_bytes * h as usize);
        for row in 0..h as usize {
            let start = row * p.stride;
            let end = start + row_bytes;
            if end > p.data.len() {
                return Err("@err_img_read|bad plane data".to_string());
            }
            buf.extend_from_slice(&p.data[start..end]);
        }
        buf
    };
    let img = image::RgbImage::from_raw(w, h, rgb)
        .ok_or_else(|| "@err_img_read|invalid buffer".to_string())?;
    Ok(DynamicImage::ImageRgb8(img))
}

/// 统一解码入口：HEIC/HEIF 走 libheif，其余走 image crate
pub(crate) fn open_image(path: &Path) -> Result<DynamicImage, String> {
    if is_heic_path(path) {
        decode_heic(path)
    } else {
        image::open(path).map_err(|e| format!("@err_img_read|{e}"))
    }
}

fn color_desc(img: &DynamicImage) -> String {
    match img.color() {
        image::ColorType::Rgb8 | image::ColorType::Rgb16 | image::ColorType::Rgb32F => "RGB".into(),
        image::ColorType::Rgba8 | image::ColorType::Rgba16 | image::ColorType::Rgba32F => "RGBA".into(),
        image::ColorType::L8 | image::ColorType::L16 => "灰度".into(),
        image::ColorType::La8 | image::ColorType::La16 => "灰度+透明".into(),
        _ => "其他".into(),
    }
}

fn ext_upper(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_else(|| "?".into())
}

fn make_thumb(img: &DynamicImage) -> Option<String> {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    let (tw, th) = if w > h {
        (256, (256u64 * h as u64 / w as u64).max(1) as u32)
    } else {
        ((256u64 * w as u64 / h as u64).max(1) as u32, 256)
    };
    let thumb = img.thumbnail(tw, th);
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80);
    if enc.encode_image(&thumb.to_rgb8()).is_err() {
        return None;
    }
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&buf)
    ))
}

/// 读取图片元信息与缩略图（仅用于预览，不写任何文件）
#[tauri::command]
pub async fn image_meta(paths: Vec<String>) -> Result<Vec<ImageMeta>, String> {
    tauri::async_runtime::spawn_blocking(move || paths.into_iter().map(|p| meta_one(&p)).collect())
        .await
        .map_err(|e| e.to_string())
}

fn meta_one(path: &str) -> ImageMeta {
    let p = Path::new(path);
    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let heic = is_heic_path(p);
    // 非 HEIC 先做尺寸上限检查（HEIC 的解码上限在 decode_heic 内完成）
    if !heic {
        if let Err(e) = check_dims(p) {
            return ImageMeta {
                path: path.to_string(),
                name,
                size,
                width: 0,
                height: 0,
                format: ext_upper(p),
                color: String::new(),
                thumb: None,
                error: Some(e),
            };
        }
    }
    match open_image(p) {
        Ok(img) => {
            let (w, h) = (img.width(), img.height());
            let thumb = make_thumb(&img);
            ImageMeta {
                path: path.to_string(),
                name,
                size,
                width: w,
                height: h,
                format: ext_upper(p),
                color: color_desc(&img),
                thumb,
                error: None,
            }
        }
        Err(e) => ImageMeta {
            path: path.to_string(),
            name,
            size,
            width: 0,
            height: 0,
            format: ext_upper(p),
            color: String::new(),
            thumb: None,
            error: Some(e),
        },
    }
}

/// 计算缩放后的尺寸
fn resize_target(img: &DynamicImage, o: &ImageOptions) -> (u32, u32) {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return (w, h);
    }
    if o.resize_mode == "percent" {
        let p = (o.percent.clamp(1, 1000) as f64) / 100.0;
        (
            ((w as f64) * p).round().max(1.0) as u32,
            ((h as f64) * p).round().max(1.0) as u32,
        )
    } else {
        let ms = o.max_side.max(16);
        if w >= h {
            (
                ms,
                (((h as u64) * (ms as u64)) / (w as u64).max(1)).max(1) as u32,
            )
        } else {
            (
                (((w as u64) * (ms as u64)) / (h as u64).max(1)).max(1) as u32,
                ms,
            )
        }
    }
}

pub(crate) fn unique_path(base: &Path, used: &mut HashSet<String>) -> PathBuf {
    let key = base.to_string_lossy().to_lowercase();
    if !used.contains(&key) && !base.exists() {
        used.insert(key);
        return base.to_path_buf();
    }
    let parent = base.parent().map(|p| p.to_path_buf());
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let ext = base
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    let mut k = 1u32;
    loop {
        let candidate = format!("{stem} ({k}){ext}");
        let cp = parent.as_deref().unwrap_or(Path::new(".")).join(&candidate);
        let ck = cp.to_string_lossy().to_lowercase();
        if !used.contains(&ck) && !cp.exists() {
            used.insert(ck);
            return cp;
        }
        k += 1;
    }
}

/// 编码输出。fmt 为 jpeg/png/webp 或「保持原格式」时的原扩展名。
pub(crate) fn encode_image(img: &DynamicImage, fmt: &str, quality: u8) -> Result<Vec<u8>, String> {
    let q = quality.clamp(1, 100);
    match fmt {
        "jpeg" | "jpg" => {
            let mut buf = Vec::new();
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q);
            enc.encode_image(&img.to_rgb8())
                .map_err(|e| format!("@err_encode_jpeg|{e}"))?;
            Ok(buf)
        }
        "webp" => {
            // 原生 libwebp（webp crate）：真正的有损压缩，支持质量参数
            let encoder = webp::Encoder::from_image(img)
                .map_err(|e| format!("@err_encode_webp|{e}"))?;
            let mem = encoder.encode(q as f32);
            Ok(mem.to_vec())
        }
        "png" => {
            let mut buf = Vec::new();
            let mut cur = std::io::Cursor::new(&mut buf);
            img.write_to(&mut cur, image::ImageFormat::Png)
                .map_err(|e| format!("@err_encode_png|{e}"))?;
            Ok(buf)
        }
        _ => {
            let f = image::ImageFormat::from_extension(fmt)
                .ok_or_else(|| format!("@err_encode_fmt|{fmt}"))?;
            let mut buf = Vec::new();
            let mut cur = std::io::Cursor::new(&mut buf);
            img.write_to(&mut cur, f)
                .map_err(|e| format!("@err_encode|{e}"))?;
            Ok(buf)
        }
    }
}

fn process_one(
    path: &str,
    o: &ImageOptions,
    out_dir: Option<&Path>,
    used: &mut HashSet<String>,
    inputs: &HashSet<String>,
) -> ImageResult {
    let src = Path::new(path);
    let name = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let in_size = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    let fail = |e: String| ImageResult {
        path: path.to_string(),
        name: name.clone(),
        out_path: None,
        out_name: None,
        in_size,
        out_size: None,
        ok: false,
        error: Some(e),
    };

    // 超大图防 OOM：非 HEIC 先读尺寸；HEIC 的解码上限在 decode_heic 内完成
    if !is_heic_path(src) {
        if let Err(e) = check_dims(src) {
            return fail(e);
        }
    }

    let img = match open_image(src) {
        Ok(i) => i,
        Err(e) => return fail(e),
    };

    // 缩放
    let img = if o.resize {
        let (w, h) = (img.width(), img.height());
        let (nw, nh) = resize_target(&img, o);
        if nw != w || nh != h {
            img.resize(nw, nh, FilterType::Lanczos3)
        } else {
            img
        }
    } else {
        img
    };

    // 输出路径
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".into());
    let ext_in = src
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "png".into());
    let heic_in = matches!(ext_in.as_str(), "heic" | "heif");
    let (fmt, ext_out) = if o.out_format == "keep" {
        if heic_in {
            // HEIC 没有可写编码器，「保持原格式」时自动转 JPEG
            ("jpeg", "jpg")
        } else {
            (ext_in.as_str(), ext_in.as_str())
        }
    } else {
        match o.out_format.as_str() {
            "jpeg" => ("jpeg", "jpg"),
            "webp" => ("webp", "webp"),
            _ => ("png", "png"),
        }
    };
    let dir: PathBuf = match out_dir {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => src
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    let same_file = {
        let candidate = dir.join(format!("{stem}.{ext_out}"));
        let a = src.canonicalize().ok();
        let b = candidate.canonicalize().ok();
        a.is_some() && a == b
    };
    let mut out_path = if same_file {
        // 目标就是源文件本身（同目录、同格式、无缩放），加后缀避免覆盖
        dir.join(format!("{stem}_compressed.{ext_out}"))
    } else {
        dir.join(format!("{stem}.{ext_out}"))
    };
    if o.overwrite {
        // overwrite 只放宽「覆盖磁盘上已存在的旧文件」；
        // 批内输出重名、以及目标恰好是批内另一个输入文件时，必须自动改名，
        // 否则会互相覆盖甚至销毁尚未处理的源文件
        let key = out_path.to_string_lossy().to_lowercase();
        if used.contains(&key) || inputs.contains(&key) {
            out_path = unique_path(&out_path, used);
        } else {
            used.insert(key);
        }
    } else {
        out_path = unique_path(&out_path, used);
    }

    let bytes = match encode_image(&img, fmt, o.quality) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    if let Err(e) = std::fs::write(&out_path, &bytes) {
        return fail(format!("@err_write|{e}"));
    }
    let out_size = bytes.len() as u64;
    let out_name = out_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned());
    ImageResult {
        path: path.to_string(),
        name,
        out_path: Some(out_path.to_string_lossy().into_owned()),
        out_name,
        in_size,
        out_size: Some(out_size),
        ok: true,
        error: None,
    }
}

/// 批量压缩/转换图片
#[tauri::command]
pub async fn process_images(
    app: AppHandle,
    job_id: String,
    paths: Vec<String>,
    options: ImageOptions,
    out_dir: Option<String>,
) -> Result<Vec<ImageResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let total = paths.len() as u32;
        let dir = out_dir.map(PathBuf::from);
        // 输出目录不存在时自动创建
        if let Some(d) = dir.as_ref().filter(|d| !d.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(d);
        }
        // 批内全部输入路径（小写），用于防止输出覆盖尚未处理的源文件
        let inputs: HashSet<String> = paths.iter().map(|p| p.to_lowercase()).collect();
        let mut used: HashSet<String> = HashSet::new();
        let mut results = Vec::with_capacity(paths.len());
        for (i, p) in paths.iter().enumerate() {
            let _ = app.emit(
                "job",
                JobProgress {
                    job_id: job_id.clone(),
                    op: "images".into(),
                    current: (i + 1) as u32,
                    total,
                    message: Some(p.clone()),
                    done: false,
                },
            );
            results.push(process_one(p, &options, dir.as_deref(), &mut used, &inputs));
        }
        let _ = app.emit(
            "job",
            JobProgress {
                job_id,
                op: "images".into(),
                current: total,
                total,
                message: None,
                done: true,
            },
        );
        results
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn sample_img() -> DynamicImage {
        let buf = RgbaImage::from_fn(64, 48, |x, y| {
            Rgba([(x * 4) as u8, (y * 5) as u8, 128, 255])
        });
        DynamicImage::ImageRgba8(buf)
    }

    #[test]
    fn encode_roundtrip_all_formats() {
        let img = sample_img();
        for fmt in ["jpeg", "webp", "png"] {
            let bytes = encode_image(&img, fmt, 80).expect(fmt);
            let decoded = image::load_from_memory(&bytes).expect("decode");
            assert_eq!((decoded.width(), decoded.height()), (64, 48), "fmt={fmt}");
        }
    }

    #[test]
    fn process_one_writes_unique_output() {
        let dir = PathBuf::from("target").join("testdata");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src_img.png");
        let img = sample_img();
        img.save(&src).unwrap();

        let o = ImageOptions {
            out_format: "webp".into(),
            quality: 80,
            resize: false,
            resize_mode: "percent".into(),
            percent: 100,
            max_side: 0,
            overwrite: false,
        };
        let mut used = HashSet::new();
        let inputs: HashSet<String> = [src.to_string_lossy().to_lowercase()].into_iter().collect();
        let r1 = process_one(&src.to_string_lossy(), &o, Some(&dir), &mut used, &inputs);
        assert!(r1.ok, "err: {:?}", r1.error);
        let r2 = process_one(&src.to_string_lossy(), &o, Some(&dir), &mut used, &inputs);
        assert!(r2.ok, "err: {:?}", r2.error);
        assert_ne!(r1.out_path, r2.out_path, "同目录重复处理必须自动改名");
    }

    #[test]
    fn process_one_overwrite_mode_still_dedups_batch() {
        let dir = PathBuf::from("target").join("testdata");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("dedup_src.png");
        let img = sample_img();
        img.save(&src).unwrap();

        // overwrite=true 允许覆盖磁盘旧文件，但同批内的输出重名必须始终自动改名
        let o = ImageOptions {
            out_format: "webp".into(),
            quality: 80,
            resize: false,
            resize_mode: "percent".into(),
            percent: 100,
            max_side: 0,
            overwrite: true,
        };
        let mut used = HashSet::new();
        let inputs: HashSet<String> = [src.to_string_lossy().to_lowercase()].into_iter().collect();
        let r1 = process_one(&src.to_string_lossy(), &o, Some(&dir), &mut used, &inputs);
        let r2 = process_one(&src.to_string_lossy(), &o, Some(&dir), &mut used, &inputs);
        assert!(r1.ok && r2.ok);
        assert_ne!(r1.out_path, r2.out_path, "overwrite 模式下批内重名也必须自动改名");
    }

    #[test]
    fn process_one_overwrite_protects_batch_inputs() {
        let dir = PathBuf::from("target").join("testdata");
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("clobber_a.png");
        let jpg = dir.join("clobber_a.jpg");
        sample_img().save(&png).unwrap();
        // 生成一个 jpg 作为「批内另一个输入文件」
        let mut jbuf = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jbuf, 90);
        enc.encode_image(&sample_img().to_rgb8()).unwrap();
        std::fs::write(&jpg, &jbuf).unwrap();

        let o = ImageOptions {
            out_format: "jpeg".into(),
            quality: 85,
            resize: false,
            resize_mode: "percent".into(),
            percent: 100,
            max_side: 0,
            overwrite: true,
        };
        let inputs: HashSet<String> = [
            png.to_string_lossy().to_lowercase(),
            jpg.to_string_lossy().to_lowercase(),
        ]
        .into_iter()
        .collect();
        let mut used = HashSet::new();
        let r = process_one(&png.to_string_lossy(), &o, Some(&dir), &mut used, &inputs);
        assert!(r.ok, "err: {:?}", r.error);
        let out = r.out_path.as_ref().unwrap();
        assert_ne!(
            out.to_lowercase(),
            jpg.to_string_lossy().to_lowercase(),
            "输出不得覆盖批内输入文件"
        );
        // 原 jpg 内容必须保持不变
        assert_eq!(std::fs::read(&jpg).unwrap(), jbuf);
    }

    #[test]
    fn heic_decode_graceful_error_and_detection() {
        let dir = PathBuf::from("target").join("testdata");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("not_heic.png");
        sample_img().save(&p).unwrap();
        // 非 HEIC 内容按 HEIC 解码必须返回错误而非 panic
        assert!(decode_heic(&p).is_err());
        // 扩展名识别
        assert!(is_heic_path(Path::new("x.HEIC")));
        assert!(is_heic_path(Path::new("x.heif")));
        assert!(!is_heic_path(Path::new("x.png")));
    }
}
