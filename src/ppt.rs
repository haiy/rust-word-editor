//! Original PresentationML generation plus preservation of user-owned presentations.
//! Packaged WPS assets retain explicit provenance; no WPS executable code is reused.
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::json;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const WIDTH: i64 = 12_192_000;
const HEIGHT: i64 = 6_858_000;
#[path = "ppt_native.rs"]
mod native;
pub use native::{render_native, template_file, template_open, template_preview};

pub fn themes() -> Result<Value> {
    let mut value: Value =
        serde_json::from_str(include_str!("ppt-theme-data.json")).context("内置主题数据无效")?;
    value["templateCatalog"] = serde_json::from_str(include_str!("ppt-template-data.json"))?;
    Ok(value)
}
fn text(value: &Value, max: usize, label: &str) -> Result<String> {
    let s = value.as_str().context(format!("{label}必须是文字"))?;
    if s.chars().count() > max {
        bail!("{label}最多 {max} 字");
    }
    if s.chars()
        .any(|c| (c as u32) < 32 && !matches!(c, '\n' | '\r' | '\t'))
    {
        bail!("{label}含无效字符");
    }
    Ok(s.to_owned())
}
pub fn validate_deck(deck: &Value) -> Result<()> {
    text(&deck["title"], 160, "演示标题")?;
    let slides = deck["slides"].as_array().context("缺少幻灯片")?;
    if slides.is_empty() || slides.len() > 50 {
        bail!("演示文稿需要 1–50 页");
    }
    for s in slides {
        text(&s["title"], 160, "页面标题")?;
        let body = s["body"].as_array().context("页面正文必须是段落数组")?;
        if body.len() > 12 {
            bail!("每页最多 12 个正文段落");
        }
        let mut n = 0;
        for p in body {
            n += text(p, 500, "正文段落")?.chars().count();
        }
        if n > 1800 {
            bail!("每页正文最多 1,800 字，请拆成多页");
        }
        if !matches!(
            s["layout"].as_str().unwrap_or("content"),
            "cover" | "content" | "section"
        ) {
            bail!("不支持的幻灯片布局");
        }
    }
    Ok(())
}
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn color(theme: &Value, key: &str, fallback: &str) -> String {
    let s = theme["colors"][key].as_str().unwrap_or(fallback);
    if s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        s.to_uppercase()
    } else {
        fallback.into()
    }
}
fn font(theme: &Value, key: &str, fallback: &str) -> String {
    theme["fonts"][key]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}
fn chosen_theme(deck: &Value) -> Result<Value> {
    let catalog = themes()?;
    let rows = catalog["themes"].as_array().context("主题目录为空")?;
    if let Some(id) = deck["themeId"].as_str() {
        return rows
            .iter()
            .find(|t| t["id"] == id)
            .cloned()
            .context("未找到这个主题");
    }
    rows.iter()
        .find(|t| t["name"] == "WPS2019")
        .or_else(|| rows.first())
        .cloned()
        .context("没有可用主题")
}
fn group() -> &'static str {
    "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>"
}
fn header(root: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><p:{root} xmlns:p=\"{P}\" xmlns:a=\"{A}\" xmlns:r=\"{R}\">")
}
fn box_shape(id: usize, x: i64, y: i64, w: i64, h: i64, fill: &str, geometry: &str) -> String {
    format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"装饰形状{id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"{geometry}\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"{fill}\"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp>",x*12700,y*12700,w*12700,h*12700)
}
// Geometry and text style map directly to independent OOXML fields.
#[allow(clippy::too_many_arguments)]
fn text_shape(
    id: usize,
    name: &str,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    paragraphs: &[String],
    size: i64,
    fill: &str,
    face: &str,
    ea: &str,
    bold: bool,
    bullets: bool,
) -> String {
    let mut s = format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap=\"square\" lIns=\"0\" rIns=\"0\" tIns=\"0\" bIns=\"0\" anchor=\"t\"><a:normAutofit/></a:bodyPr><a:lstStyle/>",esc(name),x*12700,y*12700,w*12700,h*12700);
    for paragraph in paragraphs {
        let bullet = if bullets {
            " marL=\"285750\" indent=\"-171450\""
        } else {
            ""
        };
        s.push_str(&format!("<a:p><a:pPr{bullet}><a:lnSpc><a:spcPct val=\"135000\"/></a:lnSpc><a:spcAft><a:spcPts val=\"900\"/></a:spcAft>"));
        if bullets {
            s.push_str("<a:buFont typeface=\"Arial\"/><a:buChar char=\"•\"/>");
        } else {
            s.push_str("<a:buNone/>");
        }
        s.push_str("</a:pPr>");
        for (i, line) in paragraph.replace("\r\n", "\n").split('\n').enumerate() {
            if i > 0 {
                s.push_str("<a:br/>");
            }
            s.push_str(&format!("<a:r><a:rPr lang=\"zh-CN\" sz=\"{}\" b=\"{}\"><a:solidFill><a:srgbClr val=\"{fill}\"/></a:solidFill><a:latin typeface=\"{}\"/><a:ea typeface=\"{}\"/></a:rPr><a:t>{}</a:t></a:r>",size*100,if bold {1}else{0},esc(face),esc(ea),esc(line)));
        }
        s.push_str(&format!(
            "<a:endParaRPr lang=\"zh-CN\" sz=\"{}\"/></a:p>",
            size * 100
        ));
    }
    if paragraphs.is_empty() {
        s.push_str("<a:p><a:endParaRPr lang=\"zh-CN\"/></a:p>");
    }
    s.push_str("</p:txBody></p:sp>");
    s
}
fn slide_xml(slide: &Value, theme: &Value, index: usize, count: usize) -> String {
    let accent = color(theme, "accent1", "235BAB");
    let light = color(theme, "lt1", "FFFFFF");
    let dark = color(theme, "dk1", "172B42");
    let title_font = font(theme, "major", "Arial");
    let body_font = font(theme, "minor", "Arial");
    let ea = font(theme, "minorEastAsia", "Microsoft YaHei");
    let cover = slide["layout"].as_str() == Some("cover");
    let section = slide["layout"].as_str() == Some("section");
    let (bg, fg) = if cover || section {
        (&accent, &light)
    } else {
        (&light, &dark)
    };
    let title = slide["title"].as_str().unwrap_or("").to_owned();
    let body: Vec<String> = slide["body"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_owned())
        .collect();
    let mut xml = header("sld");
    xml.push_str(&format!("<p:cSld name=\"{}\"><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"{bg}\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{}",esc(&title),group()));
    if cover || section {
        xml.push_str(&box_shape(
            2,
            800,
            0,
            160,
            540,
            &color(theme, "accent2", "D7E4F4"),
            "rect",
        ));
        xml.push_str(&text_shape(
            3,
            "页面标题",
            64,
            if section { 190 } else { 150 },
            690,
            140,
            &[title],
            if section { 40 } else { 44 },
            fg,
            &title_font,
            &ea,
            true,
            false,
        ));
        xml.push_str(&text_shape(
            4,
            "页面说明",
            68,
            320,
            670,
            150,
            &body,
            22,
            fg,
            &body_font,
            &ea,
            false,
            false,
        ));
    } else {
        xml.push_str(&box_shape(2, 0, 0, 12, 540, &accent, "rect"));
        xml.push_str(&box_shape(3, 62, 126, 836, 3, &accent, "rect"));
        xml.push_str(&text_shape(
            4,
            "页面标题",
            62,
            50,
            835,
            78,
            &[title],
            32,
            &accent,
            &title_font,
            &ea,
            true,
            false,
        ));
        let chars: usize = body.iter().map(|p| p.chars().count()).sum();
        let size = if chars > 1000 {
            13
        } else if chars > 650 {
            16
        } else if chars > 350 {
            19
        } else {
            22
        };
        xml.push_str(&text_shape(
            5,
            "页面正文",
            70,
            166,
            818,
            302,
            &body,
            size,
            fg,
            &body_font,
            &ea,
            false,
            true,
        ));
    }
    xml.push_str(&text_shape(
        6,
        "页码",
        64,
        502,
        800,
        22,
        &[format!("Rust Word · {:02} / {:02}", index + 1, count)],
        11,
        fg,
        &body_font,
        &ea,
        false,
        false,
    ));
    xml.push_str("</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>");
    xml
}
fn theme_xml(theme: &Value) -> String {
    let keys = [
        ("dk1", "172B42"),
        ("lt1", "FFFFFF"),
        ("dk2", "304965"),
        ("lt2", "EEF3F9"),
        ("accent1", "235BAB"),
        ("accent2", "5194A8"),
        ("accent3", "8A7099"),
        ("accent4", "A77B43"),
        ("accent5", "598667"),
        ("accent6", "956262"),
        ("hlink", "0563C1"),
        ("folHlink", "954F72"),
    ];
    let mut s=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><a:theme xmlns:a=\"{A}\" name=\"{}\"><a:themeElements><a:clrScheme name=\"{}\">",esc(theme["name"].as_str().unwrap_or("Rust Word")),esc(theme["name"].as_str().unwrap_or("Rust Word")));
    for (key, fallback) in keys {
        s.push_str(&format!(
            "<a:{key}><a:srgbClr val=\"{}\"/></a:{key}>",
            color(theme, key, fallback)
        ));
    }
    s.push_str("</a:clrScheme><a:fontScheme name=\"主题字体\">");
    for (tag, key) in [("majorFont", "major"), ("minorFont", "minor")] {
        s.push_str(&format!("<a:{tag}><a:latin typeface=\"{}\"/><a:ea typeface=\"{}\"/><a:cs typeface=\"Arial\"/></a:{tag}>",esc(&font(theme,key,"Arial")),esc(&font(theme,&format!("{key}EastAsia"),"Microsoft YaHei"))));
    }
    s.push_str("</a:fontScheme><a:fmtScheme name=\"Rust Word\"><a:fillStyleLst>");
    for _ in 0..3 {
        s.push_str("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>");
    }
    s.push_str("</a:fillStyleLst><a:lnStyleLst>");
    for w in [9525, 25400, 38100] {
        s.push_str(&format!("<a:ln w=\"{w}\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>"));
    }
    s.push_str("</a:lnStyleLst><a:effectStyleLst>");
    for _ in 0..3 {
        s.push_str("<a:effectStyle><a:effectLst/></a:effectStyle>");
    }
    s.push_str("</a:effectStyleLst><a:bgFillStyleLst>");
    for _ in 0..3 {
        s.push_str("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>");
    }
    s.push_str("</a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>");
    s
}
fn relationships(items: &[(String, String, String)]) -> String {
    let mut s =
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"{REL}\">");
    for (id, kind, target) in items {
        s.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{}\"/>",
            esc(target)
        ));
    }
    s.push_str("</Relationships>");
    s
}
fn write_part(zip: &mut ZipWriter<Cursor<Vec<u8>>>, path: &str, bytes: &[u8]) -> Result<()> {
    zip.start_file(
        path,
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
    )?;
    zip.write_all(bytes)?;
    Ok(())
}
pub fn export_pptx(deck: &Value) -> Result<Vec<u8>> {
    if deck["native"].as_bool() == Some(true) {
        return native::export(deck);
    }
    validate_deck(deck)?;
    let theme = chosen_theme(deck)?;
    let slides = deck["slides"].as_array().unwrap();
    let mut z = ZipWriter::new(Cursor::new(Vec::new()));
    let mut types=String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"json\" ContentType=\"application/json\"/>");
    for (path, kind) in [
        (
            "ppt/presentation.xml",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
        ),
        (
            "ppt/slideMasters/slideMaster1.xml",
            "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml",
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml",
            "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml",
        ),
        (
            "ppt/theme/theme1.xml",
            "application/vnd.openxmlformats-officedocument.theme+xml",
        ),
        (
            "docProps/core.xml",
            "application/vnd.openxmlformats-package.core-properties+xml",
        ),
        (
            "docProps/app.xml",
            "application/vnd.openxmlformats-officedocument.extended-properties+xml",
        ),
    ] {
        types.push_str(&format!(
            "<Override PartName=\"/{path}\" ContentType=\"{kind}\"/>"
        ));
    }
    for i in 0..slides.len() {
        types.push_str(&format!("<Override PartName=\"/ppt/slides/slide{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>",i+1));
    }
    types.push_str("</Types>");
    write_part(&mut z, "[Content_Types].xml", types.as_bytes())?;
    write_part(&mut z,"_rels/.rels",relationships(&[("rId1".into(),format!("{R}/officeDocument"),"ppt/presentation.xml".into()),("rId2".into(),"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties".into(),"docProps/core.xml".into()),("rId3".into(),format!("{R}/extended-properties"),"docProps/app.xml".into())]).as_bytes())?;
    let core=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:creator>Rust Word</dc:creator></cp:coreProperties>",esc(deck["title"].as_str().unwrap()));
    write_part(&mut z, "docProps/core.xml", core.as_bytes())?;
    let app=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Rust Word</Application><PresentationFormat>宽屏</PresentationFormat><Slides>{}</Slides><Notes>0</Notes><HiddenSlides>0</HiddenSlides></Properties>",slides.len());
    write_part(&mut z, "docProps/app.xml", app.as_bytes())?;
    let mut presentation = header("presentation");
    presentation.push_str("<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>");
    let mut rels = vec![
        (
            "rId1".into(),
            format!("{R}/slideMaster"),
            "slideMasters/slideMaster1.xml".into(),
        ),
        (
            "rId2".into(),
            "urn:rust-word:editable-deck".into(),
            "rustWord.json".into(),
        ),
    ];
    for i in 0..slides.len() {
        presentation.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            256 + i,
            i + 3
        ));
        rels.push((
            format!("rId{}", i + 3),
            format!("{R}/slide"),
            format!("slides/slide{}.xml", i + 1),
        ));
    }
    presentation.push_str(&format!("</p:sldIdLst><p:sldSz cx=\"{WIDTH}\" cy=\"{HEIGHT}\" type=\"screen16x9\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/><p:defaultTextStyle/></p:presentation>"));
    write_part(&mut z, "ppt/presentation.xml", presentation.as_bytes())?;
    write_part(
        &mut z,
        "ppt/_rels/presentation.xml.rels",
        relationships(&rels).as_bytes(),
    )?;
    let master=format!("{}<p:cSld name=\"原创空白母版\"><p:spTree>{}</p:spTree></p:cSld><p:clrMap accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" bg1=\"lt1\" bg2=\"lt2\" folHlink=\"folHlink\" hlink=\"hlink\" tx1=\"dk1\" tx2=\"dk2\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>",header("sldMaster"),group());
    write_part(
        &mut z,
        "ppt/slideMasters/slideMaster1.xml",
        master.as_bytes(),
    )?;
    write_part(
        &mut z,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        relationships(&[
            (
                "rId1".into(),
                format!("{R}/slideLayout"),
                "../slideLayouts/slideLayout1.xml".into(),
            ),
            (
                "rId2".into(),
                format!("{R}/theme"),
                "../theme/theme1.xml".into(),
            ),
        ])
        .as_bytes(),
    )?;
    let layout=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><p:sldLayout xmlns:p=\"{P}\" xmlns:a=\"{A}\" xmlns:r=\"{R}\" type=\"blank\" preserve=\"1\"><p:cSld name=\"原创自由布局\"><p:spTree>{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>",group());
    write_part(
        &mut z,
        "ppt/slideLayouts/slideLayout1.xml",
        layout.as_bytes(),
    )?;
    write_part(
        &mut z,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        relationships(&[(
            "rId1".into(),
            format!("{R}/slideMaster"),
            "../slideMasters/slideMaster1.xml".into(),
        )])
        .as_bytes(),
    )?;
    write_part(&mut z, "ppt/theme/theme1.xml", theme_xml(&theme).as_bytes())?;
    for (i, slide) in slides.iter().enumerate() {
        write_part(
            &mut z,
            &format!("ppt/slides/slide{}.xml", i + 1),
            slide_xml(slide, &theme, i, slides.len()).as_bytes(),
        )?;
        write_part(
            &mut z,
            &format!("ppt/slides/_rels/slide{}.xml.rels", i + 1),
            relationships(&[(
                "rId1".into(),
                format!("{R}/slideLayout"),
                "../slideLayouts/slideLayout1.xml".into(),
            )])
            .as_bytes(),
        )?;
    }
    let bytes = z.finish()?.into_inner();
    let mut parts = read_parts(&bytes)?;
    let digests: BTreeMap<_, _> = parts
        .iter()
        .map(|(name, data)| (name.clone(), digest(data)))
        .collect();
    parts.insert(
        "ppt/rustWord.json".into(),
        serde_json::to_vec(&json!({"version":1,"deck":deck,"parts":digests}))?,
    );
    pack_parts(&parts)
}
pub fn export_images(req: &Value) -> Result<Vec<u8>> {
    let images = req["images"].as_array().context("缺少逐页图片")?;
    if images.is_empty() || images.len() > 50 {
        bail!("需要 1–50 张幻灯片图片");
    }
    let mut z = ZipWriter::new(Cursor::new(Vec::new()));
    let mut total = 0;
    for (i, image) in images.iter().enumerate() {
        let s = image["data"].as_str().context("图片必须是PNG数据")?;
        let encoded = s
            .strip_prefix("data:image/png;base64,")
            .context("仅支持PNG幻灯片图片")?;
        if encoded.len() > 28 * 1024 * 1024 {
            bail!("单张图片过大");
        }
        let bytes = STANDARD.decode(encoded).context("PNG编码无效")?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() < 24 {
            bail!("无效的PNG图片");
        }
        total += bytes.len();
        if total > 160 * 1024 * 1024 {
            bail!("图片合集过大，请减少页数");
        }
        write_part(&mut z, &format!("slide-{:02}.png", i + 1), &bytes)?;
    }
    Ok(z.finish()?.into_inner())
}
/// Lossless reopen of files produced here; foreign files require a dedicated
/// import contract so complex graphics are never silently discarded.
pub fn import_own_pptx(bytes: &[u8]) -> Result<Value> {
    let parts = read_parts(bytes)?;
    let Some(raw) = parts.get("ppt/rustWord.json") else {
        return native::import(bytes, "导入演示", None);
    };
    let envelope: Value = serde_json::from_slice(raw)?;
    if envelope["version"] != 1 || !envelope["parts"].is_object() {
        bail!("此文件使用旧版无校验快照，无法确认是否经过外部修改。请在原生 PPT 应用继续编辑，避免覆盖真实内容。");
    }
    let expected = envelope["parts"].as_object().unwrap();
    if parts.len() != expected.len() + 1
        || expected.iter().any(|(name, hash)| {
            parts.get(name).map(|data| digest(data)).as_deref() != hash.as_str()
        })
    {
        bail!("检测到 PPTX 在外部应用修改，编辑快照已过期。已停止载入以保留这些修改；请在原生 PPT 应用继续编辑。");
    }
    let deck = envelope["deck"].clone();
    validate_deck(&deck)?;
    Ok(deck)
}
fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn read_parts(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    if bytes.len() > 64 * 1024 * 1024 {
        bail!("PPTX文件过大");
    }
    let mut zip = ZipArchive::new(Cursor::new(bytes)).context("无效的PPTX压缩包")?;
    if zip.len() > 10000 {
        bail!("PPTX部件过多");
    }
    let mut parts = BTreeMap::new();
    let mut total = 0;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        if name.starts_with('/') || name.split('/').any(|s| s == "..") || parts.contains_key(&name)
        {
            bail!("无效或重复的PPTX部件");
        }
        if entry.size() > 96 * 1024 * 1024 {
            bail!("PPTX部件过大");
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        total += data.len();
        if total > 256 * 1024 * 1024 {
            bail!("PPTX解压内容过大");
        }
        parts.insert(name, data);
    }
    Ok(parts)
}
fn pack_parts(parts: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in parts {
        write_part(&mut zip, name, data)?;
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pptx_native_text_and_relationships_are_well_formed() {
        let deck = json!({"title":"测试 & 可编辑","slides":[{"id":"s1","title":"封面 & 标题","body":["这是真实原生文字"],"layout":"cover"},{"id":"s2","title":"工作安排","body":["第一项","第二项"],"layout":"content"}]});
        let bytes = export_pptx(&deck).unwrap();
        assert_eq!(import_own_pptx(&bytes).unwrap(), deck);
        let mut z = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names: Vec<_> = z.file_names().map(str::to_owned).collect();
        for name in &names {
            if name.ends_with(".xml") || name.ends_with(".rels") {
                let mut raw = String::new();
                z.by_name(name).unwrap().read_to_string(&mut raw).unwrap();
                roxmltree::Document::parse(&raw).unwrap();
            }
        }
        let mut raw = String::new();
        z.by_name("ppt/slides/slide2.xml")
            .unwrap()
            .read_to_string(&mut raw)
            .unwrap();
        assert!(raw.contains("<a:t>第一项</a:t>") && raw.contains("<a:buChar"));
        assert!(!raw.contains("<p:pic>"));
        assert!(names.contains(&"ppt/theme/theme1.xml".to_string()));
    }
    #[test]
    fn invalid_decks_and_images_fail_before_export() {
        assert!(export_pptx(&json!({"title":"空","slides":[]})).is_err());
        assert!(export_images(&json!({"images":[{"data":"data:image/png;base64,eA=="}]})).is_err());
    }
    #[test]
    fn external_changes_never_load_stale_snapshot() {
        let deck = json!({"title":"快照保护","slides":[{"id":"s1","title":"标题","body":["实际正文"],"layout":"content"}]});
        let bytes = export_pptx(&deck).unwrap();
        let original = read_parts(&bytes).unwrap();
        for mode in 0..4 {
            let mut parts = original.clone();
            match mode {
                0 => {
                    let raw = String::from_utf8(parts["ppt/slides/slide1.xml"].clone())
                        .unwrap()
                        .replace("实际正文", "外部改动");
                    parts.insert("ppt/slides/slide1.xml".into(), raw.into_bytes());
                }
                1 => {
                    parts.insert("ppt/external.xml".into(), b"<changed/>".to_vec());
                }
                2 => {
                    parts.remove("ppt/theme/theme1.xml");
                }
                _ => {
                    parts.insert(
                        "ppt/rustWord.json".into(),
                        serde_json::to_vec(&deck).unwrap(),
                    );
                }
            }
            assert!(import_own_pptx(&pack_parts(&parts).unwrap()).is_err());
        }
    }
    #[test]
    fn all_packaged_native_templates_preserve_artwork_and_edit_actual_text() {
        let catalog = themes().unwrap();
        let rows = catalog["templateCatalog"]["templates"].as_array().unwrap();
        assert!(!rows.is_empty());
        let mut count = 0;
        for row in rows {
            if row["native"] != true {
                assert!(template_open(row["id"].as_str().unwrap()).is_err());
                continue;
            }
            let mut deck = template_open(row["id"].as_str().unwrap()).unwrap();
            let source = read_parts(
                &STANDARD
                    .decode(deck["sourceData"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
            if let Some(slot) = deck["slides"][0]["textSlots"]
                .as_array_mut()
                .unwrap()
                .first_mut()
            {
                slot["text"] = json!("本机原版模板编辑核验");
            }
            let bytes = export_pptx(&deck).unwrap();
            let exported = read_parts(&bytes).unwrap();
            for (name, data) in source.iter().filter(|(name, _)| {
                name.starts_with("ppt/media/")
                    || name.starts_with("ppt/slideMasters/")
                    || name.starts_with("ppt/slideLayouts/")
                    || name.starts_with("ppt/theme/")
            }) {
                assert_eq!(exported.get(name), Some(data), "{name}");
            }
            let reopened = import_own_pptx(&bytes).unwrap();
            assert_eq!(
                reopened["slides"].as_array().unwrap().len(),
                deck["slides"].as_array().unwrap().len()
            );
            // Export a reopened native deck again: relationship ids must remain unique.
            let twice = read_parts(&export_pptx(&reopened).unwrap()).unwrap();
            let relraw = std::str::from_utf8(&twice["ppt/_rels/presentation.xml.rels"]).unwrap();
            let doc = roxmltree::Document::parse(relraw).unwrap();
            let ids: Vec<_> = doc
                .descendants()
                .filter_map(|n| n.attribute("Id"))
                .collect();
            let unique: std::collections::BTreeSet<_> = ids.iter().copied().collect();
            assert_eq!(ids.len(), unique.len());
            count += 1;
        }
        assert_eq!(count, rows.len());
    }
}
