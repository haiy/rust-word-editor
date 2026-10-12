//! Bounded in-memory image editing and macOS PDFKit/Vision conversion.
//! This module does not read arbitrary user paths or show application windows.
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{codecs::jpeg::JpegEncoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Rgb};
use serde_json::{json, Value};
use std::{
    io::{Cursor, Read, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_PIXELS: u64 = 24_000_000;
const MAX_RESPONSE: usize = 192 * 1024 * 1024;

fn string<'a>(request: &'a Value, key: &str) -> &'a str {
    request.get(key).and_then(Value::as_str).unwrap_or("")
}
fn integer(request: &Value, key: &str, fallback: i64, min: i64, max: i64) -> Result<i64> {
    let number = match request.get(key) {
        None | Some(Value::Null) => fallback,
        Some(value) => value.as_i64().ok_or_else(|| anyhow!("{key} 必须是整数"))?,
    };
    if !(min..=max).contains(&number) {
        bail!("{key} 超出支持范围 {min}–{max}");
    }
    Ok(number)
}
fn number(request: &Value, key: &str, fallback: f64, min: f64, max: f64) -> Result<f64> {
    let number = match request.get(key) {
        None | Some(Value::Null) => fallback,
        Some(value) => value.as_f64().ok_or_else(|| anyhow!("{key} 必须是数字"))?,
    };
    if !number.is_finite() || !(min..=max).contains(&number) {
        bail!("{key} 超出支持范围");
    }
    Ok(number)
}
fn payload(value: &Value) -> Result<Vec<u8>> {
    let data = value.as_str().context("需要 base64 媒体数据")?;
    let encoded = if data.starts_with("data:") {
        let (header, encoded) = data.split_once(',').context("媒体 data URL 无效")?;
        if !header.ends_with(";base64") {
            bail!("媒体数据必须使用 base64");
        }
        encoded
    } else {
        data
    };
    if encoded.len() > MAX_BYTES.div_ceil(3) * 4 {
        bail!("单个媒体文件不能超过 64 MB");
    }
    let bytes = STANDARD.decode(encoded).context("媒体 base64 数据损坏")?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        bail!("媒体文件为空或超过 64 MB");
    }
    Ok(bytes)
}
fn check_size(width: u32, height: u32) -> Result<()> {
    if width == 0
        || height == 0
        || width > 12000
        || height > 12000
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        bail!("图片尺寸不能超过 12000 像素边长或 2400 万像素");
    }
    Ok(())
}
fn decode_image(bytes: &[u8]) -> Result<(DynamicImage, ImageFormat)> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let format = reader.format().context("不支持的图片格式")?;
    if ![
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::WebP,
        ImageFormat::Gif,
        ImageFormat::Bmp,
        ImageFormat::Tiff,
    ]
    .contains(&format)
    {
        bail!("支持 PNG、JPEG、WebP、GIF、BMP 和 TIFF 图片");
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12000);
    limits.max_image_height = Some(12000);
    limits.max_alloc = Some(192 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().context("图片无法解码")?;
    let (width, height) = decoder.dimensions();
    check_size(width, height)?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).context("图片内容损坏")?;
    image.apply_orientation(orientation);
    Ok((image, format))
}
fn encode_image(
    image: &DynamicImage,
    format: &str,
    quality: u8,
) -> Result<(Vec<u8>, &'static str)> {
    let mut bytes = Cursor::new(Vec::new());
    let mime = match format {
        "png" => {
            image.write_to(&mut bytes, ImageFormat::Png)?;
            "image/png"
        }
        "jpeg" | "jpg" => {
            // JPEG cannot store alpha. Composite over white instead of introducing black edges.
            let rgba = image.to_rgba8();
            let mut rgb = image::RgbImage::new(image.width(), image.height());
            for (src, dest) in rgba.pixels().zip(rgb.pixels_mut()) {
                let alpha = u16::from(src[3]);
                *dest = Rgb([0, 1, 2].map(|i| {
                    ((u16::from(src[i]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
                }));
            }
            JpegEncoder::new_with_quality(&mut bytes, quality).encode_image(&rgb)?;
            "image/jpeg"
        }
        "webp" => {
            // image's WebP encoder is lossless; the JPEG quality slider does not apply.
            DynamicImage::ImageRgba8(image.to_rgba8()).write_to(&mut bytes, ImageFormat::WebP)?;
            "image/webp"
        }
        _ => bail!("导出格式必须是 PNG、JPEG 或 WebP"),
    };
    let bytes = bytes.into_inner();
    if bytes.len() > MAX_BYTES {
        bail!("导出图片超过 64 MB，请先缩小尺寸");
    }
    Ok((bytes, mime))
}
fn image_action(action: &str, request: &Value) -> Result<Value> {
    let input = payload(&request["data"])?;
    let (mut image, source_format) = decode_image(&input)?;
    if action == "imageInspect" {
        let mime = match source_format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::WebP => "image/webp",
            ImageFormat::Gif => "image/gif",
            ImageFormat::Bmp => "image/bmp",
            ImageFormat::Tiff => "image/tiff",
            _ => unreachable!(),
        };
        // Normalize to PNG for browser rendering of TIFF/BMP, first GIF frame and EXIF orientation.
        let (bytes, _) = encode_image(&image, "png", 90)?;
        return Ok(
            json!({"data":format!("data:image/png;base64,{}",STANDARD.encode(bytes)),
            "width":image.width(),"height":image.height(),"sourceMime":mime,"sourceBytes":input.len()}),
        );
    }
    let operations = match request.get("operations") {
        None | Some(Value::Null) => vec![],
        Some(value) => value.as_array().context("图片操作必须是列表")?.clone(),
    };
    if operations.len() > 32 {
        bail!("单次最多应用 32 个图片操作");
    }
    for operation in operations {
        image = match string(&operation, "op") {
            "crop" => {
                let x = integer(&operation, "x", 0, 0, i64::from(image.width()) - 1)? as u32;
                let y = integer(&operation, "y", 0, 0, i64::from(image.height()) - 1)? as u32;
                let width = integer(
                    &operation,
                    "width",
                    i64::from(image.width() - x),
                    1,
                    i64::from(image.width() - x),
                )? as u32;
                let height = integer(
                    &operation,
                    "height",
                    i64::from(image.height() - y),
                    1,
                    i64::from(image.height() - y),
                )? as u32;
                image.crop_imm(x, y, width, height)
            }
            "rotate" => match integer(&operation, "degrees", 90, -360, 360)?.rem_euclid(360) {
                0 => image,
                90 => image.rotate90(),
                180 => image.rotate180(),
                270 => image.rotate270(),
                _ => bail!("图片旋转只支持 90° 的倍数"),
            },
            "flip" => match string(&operation, "axis") {
                "horizontal" => image.fliph(),
                "vertical" => image.flipv(),
                _ => bail!("翻转方向必须是 horizontal 或 vertical"),
            },
            "resize" => {
                let width =
                    integer(&operation, "width", i64::from(image.width()), 1, 12000)? as u32;
                let height =
                    integer(&operation, "height", i64::from(image.height()), 1, 12000)? as u32;
                check_size(width, height)?;
                image.resize_exact(width, height, image::imageops::FilterType::Lanczos3)
            }
            "brightness" => image
                .brighten((number(&operation, "value", 0., -100., 100.)? * 2.55).round() as i32),
            "contrast" => {
                image.adjust_contrast(number(&operation, "value", 0., -100., 100.)? as f32)
            }
            "grayscale" => image.grayscale(),
            _ => bail!("未知图片操作"),
        };
    }
    let format = if string(request, "format").is_empty() {
        "png"
    } else {
        string(request, "format")
    };
    let quality = integer(request, "quality", 90, 1, 100)? as u8;
    let (bytes, mime) = encode_image(&image, format, quality)?;
    Ok(
        json!({"data":format!("data:{mime};base64,{}",STANDARD.encode(&bytes)),"bytes":bytes.len(),
              "width":image.width(),"height":image.height(),"mime":mime,
              "format":if format=="jpg"{"jpeg"}else{format},"lossless":format!="jpeg"&&format!="jpg"}),
    )
}
fn validate_pdf(bytes: &[u8]) -> Result<()> {
    // PDF headers may follow a short compatibility prefix. PDFKit validates the full object structure.
    if !bytes
        .iter()
        .take(1024)
        .copied()
        .collect::<Vec<_>>()
        .windows(5)
        .any(|w| w == b"%PDF-")
    {
        bail!("文件不是有效的 PDF");
    }
    Ok(())
}
fn input_array(
    request: &Value,
    key: &str,
    min: usize,
    max: usize,
    pdf: bool,
) -> Result<Vec<String>> {
    let values = request
        .get(key)
        .and_then(Value::as_array)
        .context(format!("需要 {key} 列表"))?;
    if !(min..=max).contains(&values.len()) {
        bail!("{key} 数量超出支持范围");
    }
    let mut total = 0usize;
    let mut result = Vec::new();
    for value in values {
        let bytes = payload(value)?;
        total += bytes.len();
        if total > 128 * 1024 * 1024 {
            bail!("媒体文件总量不能超过 128 MB");
        }
        if pdf {
            validate_pdf(&bytes)?;
            result.push(STANDARD.encode(bytes));
        } else {
            let (image, _) = decode_image(&bytes)?;
            let (png, _) = encode_image(&image, "png", 90)?;
            result.push(STANDARD.encode(png));
        }
    }
    Ok(result)
}
fn native_request(action: &str, request: &Value) -> Result<Value> {
    if request.get("path").is_some() || request.get("outputPath").is_some() {
        bail!("媒体操作只接收内存数据，请使用应用文件选择器");
    }
    let mut sanitized = json!({"action":action});
    match action {
        "pdfMerge" => {
            sanitized["documents"] = json!(input_array(request, "documents", 2, 20, true)?)
        }
        "imagesToPdf" => {
            sanitized["images"] = json!(input_array(request, "images", 1, 30, false)?);
            let page_size = string(request, "pageSize");
            if !["", "a4", "original"].contains(&page_size) {
                bail!("纸张尺寸必须是 a4 或 original");
            }
            sanitized["pageSize"] = json!(if page_size.is_empty() {
                "a4"
            } else {
                page_size
            });
        }
        _ => {
            let bytes = payload(&request["data"])?;
            validate_pdf(&bytes)?;
            sanitized["data"] = json!(STANDARD.encode(bytes));
        }
    }
    match action {
        "pdfInspect" => {
            if let Some(pages) = request.get("previewPages") {
                validate_page_indices(pages, 4)?;
                sanitized["previewPages"] = pages.clone();
            }
        }
        "pdfRender" | "pdfToText" => {
            if let Some(pages) = request.get("pages") {
                validate_page_indices(pages, if action == "pdfRender" { 10 } else { 500 })?;
                sanitized["pages"] = pages.clone();
            }
            if action == "pdfRender" {
                sanitized["dpi"] = json!(number(request, "dpi", 120., 36., 300.)?);
            } else {
                let mode = string(request, "ocr");
                if !["", "auto", "always", "never"].contains(&mode) {
                    bail!("OCR 模式必须是 auto、always 或 never");
                }
                sanitized["ocr"] = json!(if mode.is_empty() { "auto" } else { mode });
            }
        }
        "pdfTransform" => {
            if let Some(pages) = request.get("pages") {
                let pages = pages.as_array().context("PDF 页码顺序必须是列表")?;
                if pages.is_empty() || pages.len() > 500 {
                    bail!("导出必须保留 1–500 页");
                }
                let mut normalized = vec![];
                for page in pages {
                    let index = integer(page, "index", -1, 0, 499)?;
                    let rotation = integer(page, "rotation", 0, -360, 360)?;
                    if rotation % 90 != 0 {
                        bail!("页面旋转只支持 90° 的倍数");
                    }
                    normalized.push(json!({"index":index,"rotation":rotation}));
                }
                sanitized["pages"] = json!(normalized);
            }
            if let Some(notes) = request.get("annotations") {
                let notes = notes.as_array().context("文字批注必须是列表")?;
                if notes.len() > 100 {
                    bail!("单次最多添加 100 条文字批注");
                }
                let mut normalized = vec![];
                for note in notes {
                    let page = integer(note, "page", -1, 0, 499)?;
                    let text = string(note, "text").trim();
                    if text.is_empty() || text.chars().count() > 2000 {
                        bail!("批注文字需要 1–2000 字");
                    }
                    let x = number(note, "x", 0.08, 0., 1.)?;
                    let y = number(note, "y", 0.08, 0., 1.)?;
                    let width = number(note, "width", 0.8, 0.01, 1.)?;
                    let height = number(note, "height", 0.12, 0.01, 1.)?;
                    if x + width > 1.000001 || y + height > 1.000001 {
                        bail!("批注不能超出页面边界");
                    }
                    let font_size = number(note, "fontSize", 14., 6., 72.)?;
                    let color = string(note, "color");
                    let color = if color.is_empty() { "#24364B" } else { color };
                    if color.len() != 7
                        || !color.starts_with('#')
                        || !color[1..].chars().all(|c| c.is_ascii_hexdigit())
                    {
                        bail!("批注颜色必须是 #RRGGBB");
                    }
                    normalized.push(
                        json!({"page":page,"text":text,"x":x,"y":y,"width":width,"height":height,
                                          "fontSize":font_size,"color":color}),
                    );
                }
                sanitized["annotations"] = json!(normalized);
            }
        }
        _ => {}
    }
    Ok(sanitized)
}
fn validate_page_indices(value: &Value, max: usize) -> Result<()> {
    let values = value.as_array().context("页码必须是列表")?;
    if values.is_empty() || values.len() > max {
        bail!("选择的页数超出支持范围，请分批处理");
    }
    if values
        .iter()
        .any(|v| !v.as_i64().is_some_and(|n| (0..500).contains(&n)))
    {
        bail!("PDF 页码必须是从 0 开始的有效整数");
    }
    Ok(())
}
fn call_native(request: &Value) -> Result<Value> {
    if !cfg!(target_os = "macos") {
        bail!("PDF 与 OCR 功能目前需要 macOS");
    }
    let executable = std::env::current_exe()?;
    let directory = executable.parent().context("找不到应用目录")?;
    let helper = directory.join("rust-word-media");
    let helper = if helper.is_file() {
        helper
    } else {
        directory
            .parent()
            .context("找不到媒体组件")?
            .join("rust-word-media")
    };
    if !helper.is_file() {
        bail!("缺少 PDF 媒体组件，请重新构建或安装完整应用");
    }
    let mut child = Command::new(helper)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("无法启动 PDF 媒体组件")?;
    let mut stdin = child.stdin.take().context("媒体输入失败")?;
    let data = serde_json::to_vec(request)?;
    let writer = thread::spawn(move || stdin.write_all(&data));
    let stdout = child.stdout.take().context("媒体输出失败")?;
    let reader = thread::spawn(move || -> Result<Vec<u8>> {
        let mut output = vec![];
        stdout
            .take((MAX_RESPONSE + 1) as u64)
            .read_to_end(&mut output)?;
        if output.len() > MAX_RESPONSE {
            bail!("媒体返回数据超出限制");
        }
        Ok(output)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            let _ = reader.join();
            bail!("PDF 处理超时，请减少页数后重试");
        }
        thread::sleep(Duration::from_millis(25));
    };
    writer.join().map_err(|_| anyhow!("媒体输入中断"))??;
    let output = reader.join().map_err(|_| anyhow!("媒体输出中断"))??;
    if !status.success() {
        bail!("PDF 媒体组件运行失败");
    }
    let result: Value = serde_json::from_slice(&output).context("媒体组件返回无效数据")?;
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        bail!("{error}");
    }
    Ok(result)
}
fn editable_text_document(result: &Value) -> Value {
    let mut content = vec![];
    if let Some(pages) = result["pages"].as_array() {
        for (index, page) in pages.iter().enumerate() {
            if index > 0 {
                content.push(json!({"type":"pageBreak"}));
            }
            let text = string(page, "text");
            for line in text.lines().filter(|s| !s.trim().is_empty()) {
                content.push(json!({"type":"paragraph","attrs":{"spaceBefore":0,"spaceAfter":8,"firstLineIndent":0,
                    "keepNext":false,"keepLines":false,"pageBreakBefore":false},"content":[{"type":"text","text":line}]}));
            }
        }
    }
    if content.is_empty() {
        content.push(json!({"type":"paragraph"}));
    }
    json!({"type":"doc","attrs":{"orientation":"portrait","marginTop":20,"marginBottom":20,
        "marginLeft":20,"marginRight":20},"content":content})
}

/// Call only after the main application has obtained bytes from an explicit user dialog.
/// `test_root` is intentionally unused: this API grants no filesystem capabilities.
pub fn dispatch(action: &str, request: &Value, _test_root: Option<&Path>) -> Result<Value> {
    match action {
        "imageInspect" | "imageEdit" => image_action(action, request),
        "pdfInspect" | "pdfRender" | "pdfToText" | "pdfTransform" | "pdfMerge" | "imagesToPdf" => {
            let sanitized = native_request(action, request)?;
            let mut result = call_native(&sanitized)?;
            if action == "pdfToText" {
                result["document"] = editable_text_document(&result);
            }
            Ok(result)
        }
        _ => bail!("未知媒体操作"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> String {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 3, |x, y| {
            image::Rgba([x as u8 * 60, y as u8 * 80, 20, 255])
        }));
        let (bytes, mime) = encode_image(&image, "png", 90).unwrap();
        format!("data:{mime};base64,{}", STANDARD.encode(bytes))
    }
    #[test]
    fn crop_rotate_and_flip_move_real_pixels() {
        let result=dispatch("imageEdit",&json!({"data":fixture(),"operations":[{"op":"crop","x":1,"y":0,"width":2,"height":3},{"op":"rotate","degrees":90},{"op":"flip","axis":"horizontal"}]}),None).unwrap();
        let (image, _) = decode_image(&payload(&result["data"]).unwrap()).unwrap();
        assert_eq!((image.width(), image.height()), (3, 2));
        // The original top-left cropped pixel reaches the output top-left after clockwise+horizontal.
        assert_eq!(image.to_rgba8().get_pixel(0, 0).0, [60, 0, 20, 255]);
    }
    #[test]
    fn jpeg_alpha_becomes_white_and_webp_stays_lossless() {
        let original = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            16,
            16,
            image::Rgba([0, 0, 0, 0]),
        ));
        let (jpeg, _) = encode_image(&original, "jpeg", 100).unwrap();
        let (decoded, _) = decode_image(&jpeg).unwrap();
        assert!(decoded.to_rgb8().get_pixel(0, 0).0.iter().all(|n| *n > 245));
        let (png, _) = decode_image(&payload(&json!(fixture())).unwrap()).unwrap();
        let (webp, _) = encode_image(&png, "webp", 90).unwrap();
        assert_eq!(decode_image(&webp).unwrap().0.to_rgba8(), png.to_rgba8());
    }
    #[test]
    fn invalid_media_requests_fail_before_any_process() {
        assert!(dispatch(
            "imageEdit",
            &json!({"data":fixture(),"operations":[{"op":"crop","x":99}]}),
            None
        )
        .is_err());
        assert!(dispatch(
            "imageEdit",
            &json!({"data":fixture(),"operations":[{"op":"resize","width":12000,"height":12000}]}),
            None
        )
        .is_err());
        assert!(dispatch(
            "pdfTransform",
            &json!({"data":"not base64","path":"/etc/passwd"}),
            None
        )
        .is_err());
        assert!(native_request("pdfTransform",&json!({"data":STANDARD.encode(b"%PDF-1.7\n"),"annotations":[{"page":0,"text":"note","x":0.9,"width":0.5}]})).is_err());
        assert!(dispatch("unknown", &json!({}), None).is_err());
    }
}
