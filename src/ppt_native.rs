//! Preserve arbitrary PresentationML objects; only explicit text slots are patched.
use super::{digest, esc, pack_parts, read_parts, text, A, P, R};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use zip::ZipArchive;

fn catalog() -> Result<Value> {
    Ok(serde_json::from_str(include_str!(
        "ppt-template-data.json"
    ))?)
}
fn row(id: &str) -> Result<Value> {
    let value = catalog()?;
    value["templates"]
        .as_array()
        .context("模板目录无效")?
        .iter()
        .find(|r| r["id"] == id)
        .cloned()
        .or_else(|| {
            value["themeAssets"]
                .as_array()
                .and_then(|rows| rows.iter().find(|r| r["id"] == id).cloned())
        })
        .context("没有找到此本机模板")
}
fn packed(entry: &str) -> Result<Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(
        include_bytes!("ppt-template-pack.zip").as_slice(),
    ))?;
    let mut bytes = Vec::new();
    zip.by_name(entry)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}
pub fn template_file(id: &str, original: bool) -> Result<Value> {
    let r = row(id)?;
    let native = !original && r["native"] == true;
    let entry = r[if native {
        "nativeEntry"
    } else {
        "originalEntry"
    }]
    .as_str()
    .context("模板文件不存在")?;
    let bytes = packed(entry)?;
    let expected = r[if native {
        "nativeSha256"
    } else {
        "sourceSha256"
    }]
    .as_str()
    .context("模板缺少校验")?;
    if digest(&bytes) != expected {
        bail!("模板资源校验失败");
    }
    let name = if native {
        format!("{}.pptx", r["name"].as_str().unwrap_or("演示模板"))
    } else {
        r["originalName"].as_str().unwrap_or("模板.ppt").to_owned()
    };
    Ok(json!({"data":STANDARD.encode(&bytes),"name":name,"bytes":bytes.len(),"native":native}))
}
pub fn template_preview(id: &str) -> Result<Value> {
    let r = row(id)?;
    if let Some(entry) = r["previewEntry"].as_str() {
        let bytes = packed(entry)?;
        let mime = if bytes.starts_with(b"\x89PNG") {
            "image/png"
        } else {
            "image/jpeg"
        };
        return Ok(
            json!({"data":format!("data:{mime};base64,{}", STANDARD.encode(bytes)),"kind":"package-thumbnail"}),
        );
    }
    Ok(json!({"data":null,"kind":"none"}))
}
pub fn template_open(id: &str) -> Result<Value> {
    let r = row(id)?;
    if r["native"] != true {
        bail!("此旧式模板受保护，无法转换或编辑。可下载完整原件，在有权访问的原生应用中打开。");
    }
    let file = template_file(id, false)?;
    let bytes = STANDARD.decode(file["data"].as_str().unwrap())?;
    let mut deck = import(&bytes, r["name"].as_str().unwrap_or("演示模板"), Some(id))?;
    deck["templateSource"] = r;
    Ok(deck)
}
fn xml_part<'a>(parts: &'a BTreeMap<String, Vec<u8>>, name: &str) -> Result<&'a str> {
    std::str::from_utf8(parts.get(name).context(format!("缺少{name}"))?).context("PPT XML编码无效")
}
fn resolve(target: &str) -> Result<String> {
    let full = if target.starts_with('/') {
        target.trim_start_matches('/').to_owned()
    } else {
        format!("ppt/{target}")
    };
    let mut stack = vec![];
    for part in full.split('/') {
        match part {
            ".." => {
                if stack.pop().is_none() {
                    bail!("PPT部件路径无效");
                }
            }
            "." | "" => (),
            _ => stack.push(part),
        }
    }
    Ok(stack.join("/"))
}
#[derive(Clone)]
struct Slot {
    start: usize,
    end: usize,
    value: String,
    virtual_slot: bool,
    prefix: String,
}
fn slots(raw: &str) -> Result<Vec<Slot>> {
    let doc = roxmltree::Document::parse(raw)?;
    let mut out = vec![];
    for n in doc.descendants().filter(|n| n.has_tag_name((A, "t"))) {
        out.push(Slot {
            start: n.range().start,
            end: n.range().end,
            value: n.text().unwrap_or("").to_owned(),
            virtual_slot: false,
            prefix: n.lookup_prefix(A).unwrap_or("a").to_owned(),
        });
    }
    // Empty placeholders remain editable without flattening their geometry or paragraph style.
    for n in doc.descendants().filter(|n| {
        n.has_tag_name((A, "p"))
            && n.ancestors().any(|p| p.has_tag_name((P, "txBody")))
            && !n.descendants().any(|t| t.has_tag_name((A, "t")))
    }) {
        let range = n.range();
        let inner = &raw[range.clone()];
        let start = if let Some(end) = n.children().find(|c| c.has_tag_name((A, "endParaRPr"))) {
            end.range().start
        } else if let Some(index) = inner.rfind("</") {
            range.start + index
        } else {
            continue;
        };
        out.push(Slot {
            start,
            end: start,
            value: String::new(),
            virtual_slot: true,
            prefix: n.lookup_prefix(A).unwrap_or("a").to_owned(),
        });
    }
    Ok(out)
}
fn patch_text(raw: &str, values: &Value) -> Result<String> {
    let positions = slots(raw)?;
    let values = values.as_array().context("缺少原生文字槽")?;
    if positions.len() != values.len() {
        bail!("文字槽数量已变更，请重新打开模板");
    }
    let mut replacements = vec![];
    for (index, (slot, value)) in positions.iter().zip(values).enumerate() {
        if value["index"].as_u64() != Some(index as u64) {
            bail!("文字槽索引无效");
        }
        let new = text(&value["text"], 4000, "模板文字")?;
        if new == slot.value {
            continue;
        }
        let a = &slot.prefix;
        let tag = if a.is_empty() {
            String::new()
        } else {
            format!("{a}:")
        };
        let replacement = if slot.virtual_slot {
            format!("<{tag}r><{tag}t>{}</{tag}t></{tag}r>", esc(&new))
        } else {
            format!("<{tag}t>{}</{tag}t>", esc(&new))
        };
        replacements.push((slot.start, slot.end, replacement));
    }
    let mut result = raw.to_owned();
    replacements.sort_by_key(|x| std::cmp::Reverse(x.0));
    for (start, end, replacement) in replacements {
        result.replace_range(start..end, &replacement);
    }
    roxmltree::Document::parse(&result)?;
    Ok(result)
}
pub fn import(bytes: &[u8], title: &str, template_id: Option<&str>) -> Result<Value> {
    let mut parts = read_parts(bytes)?;
    let presentation = xml_part(&parts, "ppt/presentation.xml")?.to_owned();
    let doc = roxmltree::Document::parse(&presentation)?;
    let dimensions = doc.descendants().find(|n| n.has_tag_name((P, "sldSz")));
    let width = dimensions
        .and_then(|n| n.attribute("cx"))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(12192000);
    let height = dimensions
        .and_then(|n| n.attribute("cy"))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(6858000);
    let relraw = xml_part(&parts, "ppt/_rels/presentation.xml.rels")?;
    let rels = roxmltree::Document::parse(relraw)?;
    let mut paths = vec![];
    for slide in doc.descendants().filter(|n| n.has_tag_name((P, "sldId"))) {
        let id = slide.attribute((R, "id")).context("页面缺少关系ID")?;
        let rel = rels
            .descendants()
            .find(|n| n.attribute("Id") == Some(id))
            .context("页面关系丢失")?;
        if rel.attribute("TargetMode") == Some("External") {
            bail!("不支持外部幻灯片引用");
        }
        paths.push(resolve(
            rel.attribute("Target").context("页面关系目标丢失")?,
        )?);
    }
    if paths.is_empty() {
        // WPS blank packages contain masters and layouts but no slide. Create a text slide
        // that uses the existing first layout; all packaged artwork stays untouched.
        let layout = parts
            .keys()
            .find(|k| {
                k.starts_with("ppt/slideLayouts/") && k.ends_with(".xml") && !k.contains("/_rels/")
            })
            .context("空白模板没有布局")?
            .clone();
        let path = "ppt/slides/rustWordBlank.xml".to_owned();
        let raw=format!("<p:sld xmlns:p=\"{P}\" xmlns:a=\"{A}\" xmlns:r=\"{R}\"><p:cSld><p:spTree>{}<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"标题\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"600000\" y=\"500000\"/><a:ext cx=\"8000000\" cy=\"1200000\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz=\"3200\"/><a:t>演示标题</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",super::group());
        parts.insert(path.clone(), raw.into_bytes());
        parts.insert(
            "ppt/slides/_rels/rustWordBlank.xml.rels".into(),
            super::relationships(&[(
                "rId1".into(),
                format!("{R}/slideLayout"),
                format!("../{}", layout.trim_start_matches("ppt/")),
            )])
            .into_bytes(),
        );
        paths.push(path);
    }
    if paths.len() > 50 {
        bail!("暂支持最多50页原版PPTX");
    }
    let mut slides = vec![];
    for (i, path) in paths.iter().enumerate() {
        let raw = xml_part(&parts, path)?;
        let text_slots = slots(raw)?;
        let title = text_slots
            .iter()
            .find(|s| !s.value.trim().is_empty())
            .map(|s| s.value.chars().take(80).collect::<String>())
            .unwrap_or_else(|| format!("第{}页", i + 1));
        slides.push(json!({"id":format!("native-{}",i+1),"sourcePart":path,"title":title,"body":[],"layout":"native","textSlots":text_slots.iter().enumerate().map(|(index,s)|json!({"index":index,"text":s.value,"emptyPlaceholder":s.virtual_slot})).collect::<Vec<_>>() }));
    }
    Ok(
        json!({"native":true,"title":title,"templateId":template_id,"sourceData":STANDARD.encode(pack_parts(&parts)?),"width":width,"height":height,"slides":slides}),
    )
}
pub fn export(deck: &Value) -> Result<Vec<u8>> {
    text(&deck["title"], 160, "演示名称")?;
    let data = deck["sourceData"].as_str().context("缺少原版演示数据")?;
    if data.len() > 90 * 1024 * 1024 {
        bail!("原版演示过大");
    }
    let mut parts = read_parts(&STANDARD.decode(data)?)?;
    let source_parts = parts.clone();
    let slides = deck["slides"].as_array().context("缺少幻灯片")?;
    if slides.is_empty() || slides.len() > 50 {
        bail!("需要1–50页幻灯片");
    }
    let mut ids = String::new();
    let mut new_rels = String::new();
    let mut type_overrides = String::new();
    for (i, slide) in slides.iter().enumerate() {
        let original = slide["sourcePart"].as_str().context("缺少原版页面来源")?;
        if !original.starts_with("ppt/slides/") || original.contains("/_rels/") {
            bail!("原版页面来源无效");
        }
        let patched = patch_text(xml_part(&source_parts, original)?, &slide["textSlots"])?;
        let new_name = format!("rustWordPage{}.xml", i + 1);
        let path = format!("ppt/slides/{new_name}");
        parts.insert(path.clone(), patched.into_bytes());
        let filename = original.rsplit('/').next().unwrap();
        let relpath = format!("ppt/slides/_rels/{filename}.rels");
        if let Some(rels) = source_parts.get(&relpath) {
            let mut relraw = std::str::from_utf8(rels)?.to_owned();
            let doc = roxmltree::Document::parse(&relraw)?;
            if let Some(note_rel) = doc
                .descendants()
                .find(|n| n.attribute("Type") == Some(&format!("{R}/notesSlide")))
            {
                let target = note_rel.attribute("Target").context("备注关系无目标")?;
                let note_path = resolve(&format!("slides/{target}"))?;
                if let Some(note) = source_parts.get(&note_path) {
                    let new_note = format!("rustWordNote{}.xml", i + 1);
                    parts.insert(format!("ppt/notesSlides/{new_note}"), note.clone());
                    let source_note_rels = format!(
                        "ppt/notesSlides/_rels/{}.rels",
                        note_path.rsplit('/').next().unwrap()
                    );
                    if let Some(note_rels) = source_parts.get(&source_note_rels) {
                        let raw = std::str::from_utf8(note_rels)?;
                        let patched = replace_relationship(
                            raw,
                            &format!("{R}/slide"),
                            &format!("../slides/{new_name}"),
                        )?;
                        parts.insert(
                            format!("ppt/notesSlides/_rels/{new_note}.rels"),
                            patched.into_bytes(),
                        );
                    }
                    type_overrides.push_str(&format!("<Override xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\" PartName=\"/ppt/notesSlides/{new_note}\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml\"/>"));
                    relraw = replace_relationship(
                        &relraw,
                        &format!("{R}/notesSlide"),
                        &format!("../notesSlides/{new_note}"),
                    )?;
                }
            }
            parts.insert(
                format!("ppt/slides/_rels/{new_name}.rels"),
                relraw.into_bytes(),
            );
        }
        ids.push_str(&format!(
            "<p:sldId xmlns:p=\"{P}\" xmlns:r=\"{R}\" id=\"{}\" r:id=\"rustWordPage{}\"/>",
            256 + i,
            i + 1
        ));
        new_rels.push_str(&format!("<Relationship xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\" Id=\"rustWordPage{}\" Type=\"{R}/slide\" Target=\"slides/{new_name}\"/>",i+1));
        type_overrides.push_str(&format!("<Override xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\" PartName=\"/{path}\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"));
    }
    let presentation = xml_part(&parts, "ppt/presentation.xml")?.to_owned();
    let doc = roxmltree::Document::parse(&presentation)?;
    let list = doc.descendants().find(|n| n.has_tag_name((P, "sldIdLst")));
    let replacement = format!("<p:sldIdLst xmlns:p=\"{P}\">{ids}</p:sldIdLst>");
    let mut patched = presentation.clone();
    if let Some(list) = list {
        patched.replace_range(list.range(), &replacement);
    } else {
        let at = patched.rfind("</").context("演示XML无效")?;
        patched.insert_str(at, &replacement);
    }
    parts.insert("ppt/presentation.xml".into(), patched.into_bytes());
    for (name, addition) in [
        ("ppt/_rels/presentation.xml.rels", new_rels),
        ("[Content_Types].xml", type_overrides),
    ] {
        let original = xml_part(&parts, name)?;
        let document = roxmltree::Document::parse(original)?;
        let mut raw = original.to_owned();
        let mut ranges: Vec<_> = document
            .descendants()
            .filter(|n| {
                if name.ends_with(".rels") {
                    n.attribute("Type") == Some(&format!("{R}/slide"))
                } else {
                    n.attribute("PartName").is_some_and(|p| {
                        p.starts_with("/ppt/slides/rustWordPage")
                            || p.starts_with("/ppt/notesSlides/rustWordNote")
                    })
                }
            })
            .map(|n| n.range())
            .collect();
        ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
        for range in ranges {
            raw.replace_range(range, "");
        }
        let at = raw.rfind("</").context("演示部件无效")?;
        let mut patched = raw;
        patched.insert_str(at, &addition);
        roxmltree::Document::parse(&patched)?;
        parts.insert(name.into(), patched.into_bytes());
    }
    // Source bytes, original slides, masters, media and unknown parts remain present.
    // No stale private model is attached: reopening uses the actual exported XML.
    parts.remove("ppt/rustWord.json");
    pack_parts(&parts)
}
fn replace_relationship(raw: &str, kind: &str, target: &str) -> Result<String> {
    let document = roxmltree::Document::parse(raw)?;
    let Some(node) = document
        .descendants()
        .find(|n| n.attribute("Type") == Some(kind))
    else {
        return Ok(raw.to_owned());
    };
    let id = node.attribute("Id").context("关系缺少ID")?;
    let replacement = format!("<Relationship xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\" Id=\"{}\" Type=\"{}\" Target=\"{}\"/>",esc(id),esc(kind),esc(target));
    let mut patched = raw.to_owned();
    patched.replace_range(node.range(), &replacement);
    Ok(patched)
}
pub fn render_native(req: &Value) -> Result<Value> {
    let deck = &req["deck"];
    if deck["native"] != true {
        bail!("此渲染接口仅用于原版PPTX");
    }
    let mut render_parts = read_parts(&export(deck)?)?;
    let mut skipped_external = 0;
    for (name, data) in render_parts
        .iter_mut()
        .filter(|(name, _)| name.ends_with(".rels"))
    {
        let raw = std::str::from_utf8(data).context(format!("{name}编码无效"))?;
        let document = roxmltree::Document::parse(raw)?;
        let mut ranges: Vec<_> = document
            .descendants()
            .filter(|n| n.attribute("TargetMode") == Some("External"))
            .map(|n| n.range())
            .collect();
        skipped_external += ranges.len();
        ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
        let mut patched = raw.to_owned();
        for range in ranges {
            patched.replace_range(range, "");
        }
        *data = patched.into_bytes();
    }
    // Only the disposable rendering copy omits external references. The user's
    // exported file keeps them; previewing never retrieves linked web resources.
    let bytes = pack_parts(&render_parts)?;
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("presentation.pptx"), bytes)?;
    let executable = std::env::current_exe()?;
    let directory = executable.parent().context("找不到应用目录")?;
    let mut candidates =
        vec![directory.join("../Resources/LibreOfficeDev.app/Contents/MacOS/soffice")];
    if let Some(path) = std::env::var_os("RUST_WORD_SOFFICE") {
        candidates.push(path.into());
    }
    candidates.push("/Applications/LibreOffice.app/Contents/MacOS/soffice".into());
    candidates.push("/Applications/LibreOfficeDev.app/Contents/MacOS/soffice".into());
    let soffice = candidates
        .into_iter()
        .find(|p| p.is_file())
        .context("未找到本机PPT渲染组件。文字编辑和PPTX导出仍可使用。")?;
    let log = fs::File::create(root.path().join("render.log"))?;
    let profile = format!(
        "-env:UserInstallation=file://{}",
        root.path().join("profile").display()
    );
    fs::create_dir_all(root.path().join("profile/user"))?;
    fs::write(
        root.path().join("profile/user/registrymodifications.xcu"),
        r#"<?xml version="1.0" encoding="UTF-8"?><oor:items xmlns:oor="http://openoffice.org/2001/registry"><item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop><prop oor:name="DisableMacrosExecution" oor:op="fuse"><value>true</value></prop></item></oor:items>"#,
    )?;
    // The portable headless build otherwise misses macOS CJK fallback fonts.
    // Font discovery and its cache belong only to this disposable render job.
    let font_config = root.path().join("fonts.conf");
    fs::write(
        &font_config,
        format!(
            r#"<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd"><fontconfig><dir>/System/Library/Fonts</dir><dir>/System/Library/Fonts/Supplemental</dir><dir>/Library/Fonts</dir><cachedir>{}</cachedir><alias><family>sans-serif</family><prefer><family>Heiti SC</family><family>Arial</family></prefer></alias></fontconfig>"#,
            esc(&root.path().join("font-cache").to_string_lossy())
        ),
    )?;
    let mut child = Command::new(soffice)
        .env("FONTCONFIG_FILE", &font_config)
        .arg(profile)
        .args([
            "--headless",
            "--nologo",
            "--nodefault",
            "--norestore",
            "--convert-to",
            "pdf",
            "--outdir",
        ])
        .arg(root.path())
        .arg(root.path().join("presentation.pptx"))
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()
        .context("无法启动PPT渲染组件")?;
    let begin = Instant::now();
    loop {
        if let Some(exit) = child.try_wait()? {
            if !exit.success() {
                bail!("原版PPT渲染失败；可导出PPTX在原生应用查看");
            }
            break;
        }
        if begin.elapsed() > Duration::from_secs(55) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("原版PPT渲染超时，请减少页数");
        }
        thread::sleep(Duration::from_millis(100));
    }
    let pdf = fs::read(root.path().join("presentation.pdf")).context("原版PPT没有生成预览")?;
    let pages: Vec<Value> = if let Some(pages) = req["pages"].as_array() {
        pages.clone()
    } else {
        (0..deck["slides"].as_array().unwrap().len())
            .map(|n| json!(n))
            .collect()
    };
    let mut images = vec![];
    for batch in pages.chunks(10) {
        let result = crate::media::dispatch(
            "pdfRender",
            &json!({"data":STANDARD.encode(&pdf),"pages":batch,"dpi":req["dpi"].as_f64().unwrap_or(120.0)}),
            None,
        )?;
        images.extend(
            result["images"]
                .as_array()
                .context("预览没有返回图片")?
                .iter()
                .cloned(),
        );
    }
    Ok(
        json!({"images":images,"renderer":"LibreOffice + macOS PDFKit","preservesArtwork":true,"externalLinksSkipped":skipped_external}),
    )
}
