//! Office Open XML import/export. All file conversion happens in Rust.
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Cursor, Read, Write},
};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MAX_ENTRY: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024;
type ListInfo = (String, i64, bool, i64);
#[derive(Clone, Default)]
struct Style {
    marks: Vec<Value>,
    paragraph: serde_json::Map<String, Value>,
    warnings: Vec<String>,
    outline: Option<i64>,
    list: Option<(String, i64)>,
    parent: Option<String>,
}
type Styles = HashMap<String, Style>;
fn merge_marks(base: &mut Vec<Value>, additions: Vec<Value>) {
    for mut mark in additions {
        let kind = sval(&mark, "type").to_string();
        if kind == "textStyle" {
            if let Some(old) = base.iter().find(|m| sval(m, "type") == kind) {
                let mut a = old["attrs"].as_object().cloned().unwrap_or_default();
                if let Some(new) = mark["attrs"].as_object() {
                    a.extend(new.clone());
                }
                mark["attrs"] = json!(a);
            }
        }
        base.retain(|m| sval(m, "type") != kind);
        let off = mark["__off"] == true;
        if let Some(obj) = mark.as_object_mut() {
            obj.remove("__off");
        }
        if !off {
            base.push(mark);
        }
    }
}
fn resolve_style(id: &str, styles: &Styles, defaults: &Style, depth: usize) -> Style {
    if depth > 30 {
        return Style::default();
    }
    let Some(own) = styles.get(id) else {
        return Style { ..defaults.clone() };
    };
    let mut s = own
        .parent
        .as_deref()
        .map(|p| resolve_style(p, styles, defaults, depth + 1))
        .unwrap_or_else(|| defaults.clone());
    merge_marks(&mut s.marks, own.marks.clone());
    s.paragraph.extend(own.paragraph.clone());
    s.warnings.extend(own.warnings.clone());
    s.outline = own.outline.or(s.outline);
    s.list = own.list.clone().or(s.list);
    s.parent = None;
    s
}

pub struct Imported {
    pub document: Value,
    pub warnings: Vec<String>,
}

fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| *c == '\n' || *c == '\r' || *c == '\t' || *c >= ' ')
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn attr<'a>(n: Node<'a, 'a>, key: &str) -> Option<&'a str> {
    n.attribute((W, key)).or_else(|| n.attribute(key))
}
fn child<'a>(n: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    n.children().find(|c| c.has_tag_name((W, name)))
}
fn sval<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn kids(v: &Value) -> &[Value] {
    v.get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn attrs(v: &Value) -> &Value {
    v.get("attrs").unwrap_or(&Value::Null)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn color_hex(value: &str, fallback: &str) -> String {
    let value = value.trim();
    let raw = value.trim_start_matches('#');
    if raw.len() == 6 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return raw.to_uppercase();
    }
    if raw.len() == 3 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return raw
            .chars()
            .flat_map(|c| [c, c])
            .collect::<String>()
            .to_uppercase();
    }
    if value.starts_with("rgb") {
        if let Some(raw) = value.split_once('(').map(|(_, r)| r.trim_end_matches(')')) {
            let parts: Vec<_> = raw
                .split([',', ' ', '/'])
                .filter(|s| !s.is_empty())
                .take(3)
                .collect();
            if parts.len() == 3 {
                let mut c = vec![];
                for p in parts {
                    let n = p.trim_end_matches('%').parse::<f64>().unwrap_or(0.);
                    c.push(
                        (if p.ends_with('%') { n * 2.55 } else { n })
                            .round()
                            .clamp(0., 255.) as u8,
                    );
                }
                return format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
            }
        }
    }
    match value.to_ascii_lowercase().as_str() {
        "black" => "000000",
        "white" => "FFFFFF",
        "red" => "FF0000",
        "blue" => "0000FF",
        "green" => "008000",
        "yellow" => "FFFF00",
        _ => fallback,
    }
    .into()
}
fn marks(v: &Value) -> &[Value] {
    v.get("marks")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn mark(v: &Value, kind: &str) -> Option<Value> {
    marks(v).iter().find(|m| sval(m, "type") == kind).cloned()
}
fn integer(v: &Value, key: &str, default: i64) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(default)
}
fn number(v: &Value, key: &str, default: f64, min: f64, max: f64) -> f64 {
    v.get(key)
        .and_then(|n| n.as_f64().or_else(|| n.as_str()?.parse().ok()))
        .filter(|n| n.is_finite())
        .unwrap_or(default)
        .clamp(min, max)
}

pub fn validate(doc: &Value) -> Result<()> {
    if sval(doc, "type") != "doc" {
        bail!("不是有效的文档结构");
    }
    fn visit(v: &Value, depth: usize, count: &mut usize) -> Result<()> {
        *count += 1;
        if depth > 80 || *count > 100_000 {
            bail!("文档结构过大或层级过深");
        }
        if ![
            "doc",
            "paragraph",
            "heading",
            "text",
            "hardBreak",
            "bulletList",
            "orderedList",
            "listItem",
            "blockquote",
            "codeBlock",
            "horizontalRule",
            "table",
            "tableRow",
            "tableCell",
            "tableHeader",
            "image",
            "pageBreak",
        ]
        .contains(&sval(v, "type"))
        {
            bail!("无法保存的内容类型：{}", sval(v, "type"));
        }
        for c in kids(v) {
            visit(c, depth + 1, count)?;
        }
        Ok(())
    }
    visit(doc, 0, &mut 0)
}

struct Writer {
    media: Vec<(String, Vec<u8>, String)>,
    rels: Vec<String>,
    lists: Vec<(i64, bool)>,
    next_id: usize,
}
impl Writer {
    fn rel(&mut self, kind: &str, target: &str, external: bool) -> String {
        self.next_id += 1;
        let id = format!("rId{}", self.next_id);
        self.rels.push(format!(
            "<Relationship Id=\"{id}\" Type=\"{R}/{kind}\" Target=\"{}\"{}/>",
            esc(target),
            if external {
                " TargetMode=\"External\""
            } else {
                ""
            }
        ));
        id
    }
    fn run(&mut self, v: &Value) -> Result<String> {
        let kind = sval(v, "type");
        if kind == "hardBreak" {
            return Ok("<w:r><w:br/></w:r>".into());
        }
        if kind == "image" {
            let src = sval(attrs(v), "src");
            let (prefix, data) = src
                .split_once(',')
                .ok_or_else(|| anyhow!("图片没有嵌入文档，请重新插入本地图片"))?;
            let (ext, mime) = match prefix {
                "data:image/png;base64" => ("png", "image/png"),
                "data:image/jpeg;base64" => ("jpg", "image/jpeg"),
                "data:image/gif;base64" => ("gif", "image/gif"),
                _ => bail!("仅支持 PNG、JPEG、GIF 内嵌图片"),
            };
            let bytes = STANDARD.decode(data).context("图片数据损坏")?;
            if bytes.len() as u64 > MAX_ENTRY {
                bail!("图片超过 32 MB");
            }
            let idx = self.media.len() + 1;
            let name = format!("image{idx}.{ext}");
            let id = self.rel("image", &format!("media/{name}"), false);
            self.media.push((name, bytes, mime.into()));
            let width = attrs(v)
                .get("width")
                .and_then(Value::as_f64)
                .unwrap_or(420.)
                .clamp(24., 650.);
            let height = attrs(v)
                .get("height")
                .and_then(Value::as_f64)
                .unwrap_or(width * 0.65)
                .clamp(24., 900.);
            let cx = (width * 9525.) as i64;
            let cy = (height * 9525.) as i64;
            return Ok(format!("<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{idx}\" name=\"图片 {idx}\" descr=\"{}\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{idx}\" name=\"图片 {idx}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",esc(sval(attrs(v),"alt"))));
        }
        let mut p = String::new();
        for m in marks(v) {
            match sval(m, "type") {
                "bold" => p.push_str("<w:b/>"),
                "italic" => p.push_str("<w:i/>"),
                "underline" => p.push_str("<w:u w:val=\"single\"/>"),
                "strike" => p.push_str("<w:strike/>"),
                "subscript" => p.push_str("<w:vertAlign w:val=\"subscript\"/>"),
                "superscript" => p.push_str("<w:vertAlign w:val=\"superscript\"/>"),
                "code" => p.push_str("<w:rFonts w:ascii=\"Menlo\" w:hAnsi=\"Menlo\"/>"),
                "highlight" => p.push_str(&format!(
                    "<w:shd w:val=\"clear\" w:fill=\"{}\"/>",
                    color_hex(sval(attrs(m), "color"), "FFF3A3")
                )),
                "textStyle" => {
                    let a = attrs(m);
                    let color = sval(a, "color");
                    let font = sval(a, "fontFamily");
                    if !color.is_empty() {
                        p.push_str(&format!(
                            "<w:color w:val=\"{}\"/>",
                            color_hex(color, "000000")
                        ));
                    }
                    if !font.is_empty() {
                        p.push_str(&format!(
                            "<w:rFonts w:ascii=\"{}\" w:hAnsi=\"{}\" w:eastAsia=\"{}\"/>",
                            esc(font),
                            esc(font),
                            esc(font)
                        ));
                    }
                    let s = sval(a, "fontSize");
                    if let Ok(n) = s
                        .trim_end_matches("pt")
                        .trim_end_matches("px")
                        .parse::<f64>()
                    {
                        let n = if s.ends_with("px") { n * 0.75 } else { n };
                        p.push_str(&format!(
                            "<w:sz w:val=\"{}\"/><w:szCs w:val=\"{}\"/>",
                            (n.clamp(6., 96.) * 2.) as i64,
                            (n.clamp(6., 96.) * 2.) as i64
                        ));
                    }
                }
                _ => {}
            }
        }
        let text = sval(v, "text")
            .split('\n')
            .map(|s| format!("<w:t xml:space=\"preserve\">{}</w:t>", esc(s)))
            .collect::<Vec<_>>()
            .join("<w:br/>");
        let run = format!("<w:r><w:rPr>{p}</w:rPr>{text}</w:r>");
        if let Some(link) = mark(v, "link") {
            let href = sval(attrs(&link), "href");
            if href.starts_with("https://")
                || href.starts_with("http://")
                || href.starts_with("mailto:")
            {
                let id = self.rel("hyperlink", href, true);
                return Ok(format!("<w:hyperlink r:id=\"{id}\">{run}</w:hyperlink>"));
            }
        }
        Ok(run)
    }
    fn blocks(&mut self, vs: &[Value], list: Option<(i64, i64)>, quote: bool) -> Result<String> {
        let mut out = String::new();
        for v in vs {
            let kind = sval(v, "type");
            let a = attrs(v);
            match kind {
                "paragraph"|"heading"|"codeBlock"=>{
                    let mut p=String::new();
                    if kind=="heading" {p.push_str(&format!("<w:pStyle w:val=\"Heading{}\"/>",integer(a,"level",1).clamp(1,6)));}
                    if quote {p.push_str("<w:pBdr><w:left w:val=\"single\" w:sz=\"18\" w:space=\"12\" w:color=\"C4CCD6\"/></w:pBdr>");}
                    let align=sval(a,"textAlign");if ["left","center","right","justify"].contains(&align){p.push_str(&format!("<w:jc w:val=\"{}\"/>",if align=="justify"{"both"}else{align}));}
                    let lh=number(a,"lineHeight",1.5,1.,3.);
                    let before=(number(a,"spaceBefore",0.,0.,96.)*20.).round() as i64;
                    let after=(number(a,"spaceAfter",8.,0.,96.)*20.).round() as i64;
                    p.push_str(&format!("<w:spacing w:before=\"{before}\" w:after=\"{after}\" w:line=\"{}\" w:lineRule=\"auto\"/>",(lh*240.).round() as i64));
                    let indent=integer(a,"indent",0).clamp(0,8);
                    let left=if quote {540} else if indent>0 {indent*360} else {list.map(|(_,depth)|720+depth*360).unwrap_or(0)};
                    let first=(number(a,"firstLineIndent",0.,-36.,72.)*20.).round() as i64;
                    if quote || list.is_none() || indent>0 || first!=0 {
                        p.push_str(&format!("<w:ind w:left=\"{left}\" {}=\"{}\"/>",if first<0{"w:hanging"}else{"w:firstLine"},first.abs()));
                    }
                    for key in ["keepNext","keepLines","pageBreakBefore"] {
                        p.push_str(&format!("<w:{key} w:val=\"{}\"/>",if a[key].as_bool().unwrap_or(false){"1"}else{"0"}));
                    }
                    if let Some((id,depth))=list{p.push_str(&format!("<w:numPr><w:ilvl w:val=\"{depth}\"/><w:numId w:val=\"{id}\"/></w:numPr>"));}
                    let mut runs=String::new();for c in kids(v){runs.push_str(&self.run(c)?);}
                    out.push_str(&format!("<w:p><w:pPr>{p}</w:pPr>{runs}</w:p>"));
                }
                "bulletList"|"orderedList"=>{
                    let depth=list.map(|(_,d)|d+1).unwrap_or(0).min(8);
                    let id=(self.lists.len()+1) as i64;self.lists.push((integer(a,"start",1),kind=="orderedList"));
                    for item in kids(v){let mut first=true;for c in kids(item){let numbered=matches!(sval(c,"type"),"paragraph"|"heading")&&first;if numbered{first=false;}
                        out.push_str(&self.blocks(std::slice::from_ref(c),if numbered||matches!(sval(c,"type"),"bulletList"|"orderedList"){Some((id,depth))}else{None},quote)?);}}
                }
                "blockquote"=>out.push_str(&self.blocks(kids(v),list,true)?),
                "horizontalRule"=>out.push_str("<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:color=\"CBD5E1\"/></w:pBdr></w:pPr></w:p>"),
                "pageBreak"=>out.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>"),
                "image"=>{let r=self.run(v)?;out.push_str(&format!("<w:p>{r}</w:p>"));},
                "table"=>{
                    let cols=kids(v).first().map(|r|kids(r).iter().map(|c|integer(attrs(c),"colspan",1).max(1) as usize).sum::<usize>()).unwrap_or(1).clamp(1,50);
                    out.push_str("<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders>");
                    for b in ["top","left","bottom","right","insideH","insideV"]{out.push_str(&format!("<w:{b} w:val=\"single\" w:sz=\"4\" w:color=\"CBD5E1\"/>"));}
                    out.push_str("</w:tblBorders></w:tblPr><w:tblGrid>");for _ in 0..cols {out.push_str(&format!("<w:gridCol w:w=\"{}\"/>",9000/cols));}out.push_str("</w:tblGrid>");
                    let mut spans=vec![(0i64,1usize);cols];
                    for row in kids(v){out.push_str("<w:tr>");let mut col=0;
                        for cell in kids(row){
                            while col<cols&&spans[col].0>0 {let (left,width)=spans[col];out.push_str(&format!("<w:tc><w:tcPr><w:gridSpan w:val=\"{width}\"/><w:vMerge/></w:tcPr><w:p/></w:tc>"));spans[col].0=left-1;col+=width;}
                            let width=integer(attrs(cell),"colspan",1).clamp(1,50) as usize;let rows=integer(attrs(cell),"rowspan",1).clamp(1,1000);
                            out.push_str(&format!("<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/><w:gridSpan w:val=\"{width}\"/>",9000/cols*width));
                            if rows>1 {out.push_str("<w:vMerge w:val=\"restart\"/>");if col<cols{spans[col]=(rows-1,width);}}
                            if sval(cell,"type")=="tableHeader" {out.push_str("<w:shd w:fill=\"EEF2F8\"/>");}
                            out.push_str("</w:tcPr>");let body=self.blocks(kids(cell),None,false)?;out.push_str(&body);if body.is_empty()||kids(cell).last().is_some_and(|n|sval(n,"type")=="table"){out.push_str("<w:p/>");}out.push_str("</w:tc>");col+=width;
                        }
                        while col<cols&&spans[col].0>0 {let(left,width)=spans[col];out.push_str(&format!("<w:tc><w:tcPr><w:gridSpan w:val=\"{width}\"/><w:vMerge/></w:tcPr><w:p/></w:tc>"));spans[col].0=left-1;col+=width;}
                        out.push_str("</w:tr>");
                    }out.push_str("</w:tbl>");
                },_=>bail!("无法导出的块：{kind}")
            }
        }
        Ok(out)
    }
}

pub fn export(document: &Value) -> Result<Vec<u8>> {
    validate(document)?;
    let mut w = Writer {
        media: vec![],
        rels: vec![],
        lists: vec![],
        next_id: 0,
    };
    w.rel("styles", "styles.xml", false);
    w.rel("numbering", "numbering.xml", false);
    let body = w.blocks(kids(document), None, false)?;
    let landscape = sval(attrs(document), "orientation") == "landscape";
    let size = if landscape {
        "w:w=\"16838\" w:h=\"11906\" w:orient=\"landscape\""
    } else {
        "w:w=\"11906\" w:h=\"16838\""
    };
    let margins = ["marginTop", "marginRight", "marginBottom", "marginLeft"]
        .map(|key| (number(attrs(document), key, 20., 5., 50.) * 1440. / 25.4).round() as i64);
    let [top, right, bottom, left] = margins;
    let xml=format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{body}<w:sectPr><w:pgSz {size}/><w:pgMar w:top=\"{top}\" w:right=\"{right}\" w:bottom=\"{bottom}\" w:left=\"{left}\" w:header=\"567\" w:footer=\"567\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>");
    let mut types=String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"json\" ContentType=\"application/json\"/>");
    for (ext, mime) in [
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("gif", "image/gif"),
    ] {
        types.push_str(&format!(
            "<Default Extension=\"{ext}\" ContentType=\"{mime}\"/>"
        ));
    }
    types.push_str("<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/></Types>");
    let mut styles=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:styles xmlns:w=\"{W}\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Arial\" w:hAnsi=\"Arial\" w:eastAsia=\"PingFang SC\"/><w:sz w:val=\"24\"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>");
    for level in 1..=6 {
        styles.push_str(&format!("<w:style w:type=\"paragraph\" w:styleId=\"Heading{level}\"><w:name w:val=\"heading {level}\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:keepNext/><w:outlineLvl w:val=\"{}\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"{}\"/></w:rPr></w:style>",level-1,[44,34,28,26,24,24][level-1]));
    }
    styles.push_str("</w:styles>");
    let mut numbering =
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:numbering xmlns:w=\"{W}\">");
    for (idx, (start, ordered)) in w.lists.iter().enumerate() {
        let id = idx + 1;
        numbering.push_str(&format!(
            "<w:abstractNum w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"multilevel\"/>"
        ));
        for depth in 0..=8 {
            numbering.push_str(&format!("<w:lvl w:ilvl=\"{depth}\"><w:start w:val=\"{start}\"/><w:numFmt w:val=\"{}\"/><w:lvlText w:val=\"{}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr></w:lvl>",if *ordered{"decimal"}else{"bullet"},if *ordered{format!("%{}.",depth+1)}else{"•".into()},720+depth*360));
        }
        numbering.push_str(&format!(
            "</w:abstractNum><w:num w:numId=\"{id}\"><w:abstractNumId w:val=\"{id}\"/></w:num>"
        ));
    }
    numbering.push_str("</w:numbering>");
    let rels=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}</Relationships>",w.rels.join(""));
    let mut hashes = serde_json::Map::new();
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name,data) in [("[Content_Types].xml",types.as_bytes()),("_rels/.rels",b"<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".as_slice()),("word/document.xml",xml.as_bytes()),("word/styles.xml",styles.as_bytes()),("word/numbering.xml",numbering.as_bytes()),("word/_rels/document.xml.rels",rels.as_bytes())]{hashes.insert(name.into(),json!(digest(data)));zip.start_file(name,options)?;zip.write_all(data)?;}
    for (name, bytes, _) in w.media {
        hashes.insert(format!("word/media/{name}"), json!(digest(&bytes)));
        zip.start_file(format!("word/media/{name}"), options)?;
        zip.write_all(&bytes)?;
    }
    let snapshot = serde_json::to_vec(&json!({"version":2,"parts":hashes,"document":document}))?;
    zip.start_file("word/rustword.json", options)?;
    zip.write_all(&snapshot)?;
    Ok(zip.finish()?.into_inner())
}

struct Parser {
    entries: HashMap<String, Vec<u8>>,
    rels: HashMap<String, (String, bool)>,
    numbering: HashMap<(String, i64), (bool, i64)>,
    warnings: Vec<String>,
}
impl Parser {
    fn warn(&mut self, s: &str) {
        if !self.warnings.iter().any(|w| w == s) {
            self.warnings.push(s.into());
        }
    }
    fn paragraph_props(&mut self, pr: Node) -> serde_json::Map<String, Value> {
        let mut a = serde_json::Map::new();
        if let Some(j) = child(pr, "jc").and_then(|n| attr(n, "val")) {
            if ["left", "center", "right", "both", "justify"].contains(&j) {
                a.insert(
                    "textAlign".into(),
                    json!(if j == "both" { "justify" } else { j }),
                );
            } else {
                self.warn("部分段落对齐方式暂不支持，另存副本以保留原文件。");
            }
        }
        if let Some(spacing) = child(pr, "spacing") {
            for (xml_key, key) in [("before", "spaceBefore"), ("after", "spaceAfter")] {
                if let Some(raw) = attr(spacing, xml_key) {
                    if let Some(n) = raw.parse::<f64>().ok().filter(|n| n.is_finite()) {
                        if !(0. ..=1920.).contains(&n) {
                            self.warn("部分段落间距超出支持范围，另存副本以保留原文件。");
                        }
                        a.insert(key.into(), json!((n / 20.).clamp(0., 96.)));
                    } else {
                        self.warn("文档包含无效段落间距，已使用默认值。");
                    }
                }
            }
            if let Some(raw) = attr(spacing, "line") {
                let rule = attr(spacing, "lineRule").unwrap_or("auto");
                if rule == "auto" {
                    if let Some(n) = raw.parse::<f64>().ok().filter(|n| n.is_finite()) {
                        if !(240. ..=720.).contains(&n) {
                            self.warn("部分行距超出支持范围，另存副本以保留原文件。");
                        }
                        a.insert(
                            "lineHeight".into(),
                            json!((n / 240.).clamp(1., 3.).to_string()),
                        );
                    } else {
                        self.warn("文档包含无效行距，已使用默认值。");
                    }
                } else {
                    self.warn("固定值或最小值行距暂不支持，编辑时转换为倍数行距。");
                }
            }
            if [
                "beforeLines",
                "afterLines",
                "beforeAutospacing",
                "afterAutospacing",
            ]
            .iter()
            .any(|key| attr(spacing, key).is_some_and(|v| !["0", "false", "off"].contains(&v)))
            {
                self.warn("按行或自动段落间距暂不支持，另存副本以保留原文件。");
            }
        }
        if let Some(ind) = child(pr, "ind") {
            if let Some(raw) = attr(ind, "left").or_else(|| attr(ind, "start")) {
                if let Some(n) = raw.parse::<f64>().ok().filter(|n| n.is_finite()) {
                    if !(0. ..=2880.).contains(&n) || (n / 360. - (n / 360.).round()).abs() > 0.001
                    {
                        self.warn("部分左缩进转换为编辑器缩进级别，另存副本以保留原文件。");
                    }
                    a.insert(
                        "indent".into(),
                        json!((n / 360.).round().clamp(0., 8.) as i64),
                    );
                } else {
                    self.warn("文档包含无效缩进，已使用默认值。");
                }
            }
            let first = attr(ind, "hanging")
                .map(|v| (v, -1.))
                .or_else(|| attr(ind, "firstLine").map(|v| (v, 1.)));
            if let Some((raw, sign)) = first {
                if let Some(n) = raw
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite() && *n >= 0.)
                {
                    let pts = sign * n / 20.;
                    if !(-36. ..=72.).contains(&pts) {
                        self.warn("部分首行或悬挂缩进超出支持范围，另存副本以保留原文件。");
                    }
                    a.insert("firstLineIndent".into(), json!(pts.clamp(-36., 72.)));
                } else {
                    self.warn("文档包含无效首行缩进，已使用默认值。");
                }
            }
            if ["leftChars", "rightChars", "firstLineChars", "hangingChars"]
                .iter()
                .any(|key| attr(ind, key).is_some())
                || ["right", "end"]
                    .iter()
                    .any(|key| attr(ind, key).is_some_and(|v| v != "0"))
            {
                self.warn("字符单位或右侧缩进暂不支持，另存副本以保留原文件。");
            }
        }
        for key in ["keepNext", "keepLines", "pageBreakBefore"] {
            if let Some(n) = child(pr, key) {
                let raw = attr(n, "val").unwrap_or("1");
                match raw {
                    "1" | "true" | "on" => {
                        a.insert(key.into(), json!(true));
                    }
                    "0" | "false" | "off" => {
                        a.insert(key.into(), json!(false));
                    }
                    _ => self.warn("文档包含无效段落分页开关，已使用默认值。"),
                }
            }
        }
        a
    }
    fn props(&self, n: Node) -> Vec<Value> {
        let mut ms = vec![];
        let mut style = serde_json::Map::new();
        for c in n.children().filter(Node::is_element) {
            let name = c.tag_name().name();
            let val = attr(c, "val").unwrap_or("");
            match name {
                "b" | "i" | "strike" => {
                    ms.push(json!({"type":match name{"b"=>"bold","i"=>"italic",_=>"strike"},"__off":(["0","false","off"].contains(&val))}));
                }
                "u" => ms.push(json!({"type":"underline","__off":val=="none"})),
                "vertAlign" if ["subscript", "superscript"].contains(&val) => {
                    ms.push(json!({"type":val}))
                }
                "color" if val != "auto" && !val.is_empty() => {
                    style.insert("color".into(), json!(format!("#{val}")));
                }
                "rFonts" => {
                    if let Some(f) = attr(c, "eastAsia").or_else(|| attr(c, "ascii")) {
                        style.insert("fontFamily".into(), json!(f));
                    }
                }
                "sz" => {
                    if let Ok(size) = val.parse::<f64>() {
                        style.insert("fontSize".into(), json!(format!("{}pt", size / 2.)));
                    }
                }
                "shd" => {
                    if let Some(fill) = attr(c, "fill").filter(|s| *s != "auto") {
                        ms.push(json!({"type":"highlight","attrs":{"color":format!("#{fill}")}}));
                    }
                }
                "highlight" => {
                    let color = match val {
                        "yellow" => "#ffff00",
                        "green" => "#00ff00",
                        "cyan" => "#00ffff",
                        "magenta" => "#ff00ff",
                        "blue" => "#0000ff",
                        "red" => "#ff0000",
                        "darkYellow" => "#808000",
                        _ => "#fff3a3",
                    };
                    ms.push(json!({"type":"highlight","attrs":{"color":color}}));
                }
                _ => {}
            }
        }
        if !style.is_empty() {
            ms.push(json!({"type":"textStyle","attrs":style}));
        }
        ms
    }
    fn inline(&mut self, n: Node, inherited: &[Value], link: Option<&str>) -> Result<Vec<Value>> {
        let mut out = vec![];
        for r in n.children().filter(Node::is_element) {
            match r.tag_name().name() {
                "r" => {
                    let mut ms = inherited.to_vec();
                    if let Some(pr) = child(r, "rPr") {
                        merge_marks(&mut ms, self.props(pr));
                    }
                    if let Some(href) = link {
                        if href.starts_with("http://")
                            || href.starts_with("https://")
                            || href.starts_with("mailto:")
                        {
                            ms.push(json!({"type":"link","attrs":{"href":href,"target":"_blank"}}));
                        }
                    }
                    for c in r.children().filter(Node::is_element) {
                        match c.tag_name().name() {
                            "t" => {
                                let t = c.text().unwrap_or("");
                                if !t.is_empty() {
                                    let mut v = json!({"type":"text","text":t});
                                    if !ms.is_empty() {
                                        v["marks"] = json!(ms);
                                    }
                                    out.push(v);
                                }
                            }
                            "tab" => out.push(json!({"type":"text","text":"\t"})),
                            "br" | "cr" => {
                                if attr(c, "type") == Some("page") {
                                    out.push(json!({"type":"__page"}));
                                } else {
                                    out.push(json!({"type":"hardBreak"}));
                                }
                            }
                            "drawing" => {
                                let image = c
                                    .descendants()
                                    .find(|n| n.tag_name().name() == "blip")
                                    .and_then(|n| n.attribute((R, "embed")));
                                if let Some(id) = image {
                                    if let Some((target, false)) = self.rels.get(id) {
                                        if let Some(bytes) = self.entries.get(target) {
                                            let ext = target
                                                .rsplit('.')
                                                .next()
                                                .unwrap_or("")
                                                .to_lowercase();
                                            let mime = match ext.as_str() {
                                                "png" => Some("image/png"),
                                                "jpg" | "jpeg" => Some("image/jpeg"),
                                                "gif" => Some("image/gif"),
                                                _ => None,
                                            };
                                            if let Some(mime) = mime {
                                                let e = c
                                                    .descendants()
                                                    .find(|n| n.tag_name().name() == "extent");
                                                let width = e
                                                    .and_then(|n| n.attribute("cx"))
                                                    .and_then(|s| s.parse::<f64>().ok())
                                                    .unwrap_or(400. * 9525.)
                                                    / 9525.;
                                                let height = e
                                                    .and_then(|n| n.attribute("cy"))
                                                    .and_then(|s| s.parse::<f64>().ok())
                                                    .unwrap_or(260. * 9525.)
                                                    / 9525.;
                                                out.push(json!({"type":"image","attrs":{"src":format!("data:{mime};base64,{}",STANDARD.encode(bytes)),"width":width,"height":height,"alt":""}}));
                                            } else {
                                                self.warn("文档包含不支持的矢量或特殊图片，已跳过；原文件保留。");
                                            }
                                        }
                                    }
                                } else {
                                    self.warn("文档包含浮动图形或外部图片，布局可能变化。");
                                }
                            }
                            "instrText" => self
                                .warn("文档包含域或自动目录；保留显示文本，自动更新功能不保留。"),
                            "pict" | "object" => {
                                self.warn("文档包含旧式图片、图形或嵌入对象，不能完整转换。")
                            }
                            _ => {}
                        }
                    }
                }
                "hyperlink" => {
                    let href = r
                        .attribute((R, "id"))
                        .and_then(|id| self.rels.get(id))
                        .map(|p| p.0.clone());
                    out.extend(self.inline(r, inherited, href.as_deref())?);
                }
                "ins" | "del" => {
                    self.warn("文档包含修订；导入当前正文，修订历史不保留。");
                    if r.tag_name().name() == "ins" {
                        out.extend(self.inline(r, inherited, link)?);
                    }
                }
                "sdt" => {
                    self.warn("文档包含内容控件；转换为普通文本。");
                    if let Some(content) = child(r, "sdtContent") {
                        out.extend(self.inline(content, inherited, link)?);
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }
    fn paragraph(&mut self, p: Node, styles: &Styles) -> Result<(Vec<Value>, Option<ListInfo>)> {
        let pr = child(p, "pPr");
        let style = pr
            .and_then(|n| child(n, "pStyle"))
            .and_then(|n| attr(n, "val"))
            .unwrap_or("Normal");
        let inherited = styles
            .get(style)
            .map(|p| p.marks.clone())
            .unwrap_or_default();
        if let Some(s) = styles.get(style) {
            for warning in &s.warnings {
                self.warn(warning);
            }
        }
        let inferred = style
            .to_ascii_lowercase()
            .strip_prefix("heading")
            .and_then(|s| s.trim().parse::<i64>().ok());
        let level = styles
            .get(style)
            .and_then(|p| p.outline)
            .or(inferred)
            .or(match style {
                "Title" => Some(1),
                "Subtitle" => Some(2),
                _ => None,
            });
        let mut a = json!(styles
            .get(style)
            .map(|s| s.paragraph.clone())
            .unwrap_or_default());
        if let Some(l) = level {
            a["level"] = json!(l.clamp(1, 6));
        }
        if let Some(pr) = pr {
            a.as_object_mut().unwrap().extend(self.paragraph_props(pr));
        }
        let direct_number = pr.and_then(|n| child(n, "numPr")).and_then(|n| {
            let id = child(n, "numId").and_then(|n| attr(n, "val"))?.to_string();
            if id == "0" {
                return None;
            }
            let depth = child(n, "ilvl")
                .and_then(|n| attr(n, "val"))
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0)
                .clamp(0, 8);
            let (ordered, start) = self
                .numbering
                .get(&(id.clone(), depth))
                .copied()
                .unwrap_or((false, 1));
            Some((id, depth, ordered, start))
        });
        let zero_number = pr
            .and_then(|n| child(n, "numPr"))
            .and_then(|n| child(n, "numId"))
            .and_then(|n| attr(n, "val"))
            == Some("0");
        let number = if zero_number {
            None
        } else {
            direct_number.or_else(|| {
                styles
                    .get(style)
                    .and_then(|s| s.list.clone())
                    .map(|(id, depth)| {
                        let (ordered, start) = self
                            .numbering
                            .get(&(id.clone(), depth))
                            .copied()
                            .unwrap_or((false, 1));
                        (id, depth, ordered, start)
                    })
            })
        };
        let inline = self.inline(p, &inherited, None)?;
        let mut blocks = vec![];
        let mut current = vec![];
        let flush = |blocks: &mut Vec<Value>, current: &mut Vec<Value>| {
            blocks.push(json!({"type":if level.is_some(){"heading"}else{"paragraph"},"attrs":a,"content":std::mem::take(current)}));
        };
        for v in inline {
            match sval(&v, "type") {
                "image" => {
                    if !current.is_empty() {
                        flush(&mut blocks, &mut current);
                    }
                    blocks.push(v);
                }
                "__page" => {
                    if !current.is_empty() {
                        flush(&mut blocks, &mut current);
                    }
                    blocks.push(json!({"type":"pageBreak"}));
                }
                _ => current.push(v),
            }
        }
        if !current.is_empty() || blocks.is_empty() {
            flush(&mut blocks, &mut current);
        }
        Ok((blocks, number))
    }
    fn body(&mut self, body: Node, styles: &Styles) -> Result<Vec<Value>> {
        let mut out = vec![];
        let mut last_num = None::<String>;
        for node in body.children().filter(Node::is_element) {
            match node.tag_name().name() {
                "p" => {
                    let (blocks, number) = self.paragraph(node, styles)?;
                    if let Some((id, depth, ordered, start)) = number {
                        let same = last_num.as_deref() == Some(&id);
                        append_list(&mut out, blocks, depth as usize, ordered, start, same);
                        last_num = Some(id);
                    } else {
                        out.extend(blocks);
                        last_num = None;
                    }
                }
                "tbl" => {
                    last_num = None;
                    let mut rows: Vec<Value> = vec![];
                    let mut origins: HashMap<usize, (usize, usize)> = HashMap::new();
                    for tr in node.children().filter(|n| n.has_tag_name((W, "tr"))) {
                        let mut cells = vec![];
                        let mut col = 0usize;
                        for tc in tr.children().filter(|n| n.has_tag_name((W, "tc"))) {
                            let pr = child(tc, "tcPr");
                            let span = pr
                                .and_then(|n| child(n, "gridSpan"))
                                .and_then(|n| attr(n, "val"))
                                .and_then(|s| s.parse::<usize>().ok())
                                .unwrap_or(1)
                                .clamp(1, 50);
                            let merge = pr.and_then(|n| child(n, "vMerge"));
                            if merge.is_some()
                                && merge.and_then(|n| attr(n, "val")) != Some("restart")
                            {
                                if let Some((r, c)) = origins.get(&col).copied() {
                                    let n = rows[r]["content"][c]["attrs"]["rowspan"]
                                        .as_u64()
                                        .unwrap_or(1);
                                    rows[r]["content"][c]["attrs"]["rowspan"] = json!(n + 1);
                                }
                            } else {
                                if merge.is_some() {
                                    origins.insert(col, (rows.len(), cells.len()));
                                } else {
                                    origins.remove(&col);
                                }
                                let mut content = self.body(tc, styles)?;
                                if content.is_empty() {
                                    content.push(json!({"type":"paragraph"}));
                                }
                                cells.push(json!({"type":"tableCell","attrs":{"colspan":span,"rowspan":1},"content":content}));
                            }
                            col += span;
                        }
                        rows.push(json!({"type":"tableRow","content":cells}));
                    }
                    out.push(json!({"type":"table","content":rows}));
                }
                "sdt" => {
                    self.warn("文档包含内容控件，已转换为普通内容。");
                    if let Some(c) = child(node, "sdtContent") {
                        out.extend(self.body(c, styles)?);
                    }
                }
                "sectPr" | "tcPr" => {}
                _ => {
                    self.warn("部分特殊文档元素无法编辑，导入时未保留。原文件不会被直接覆盖。");
                }
            }
        }
        Ok(out)
    }
}

fn append_list(
    out: &mut Vec<Value>,
    blocks: Vec<Value>,
    depth: usize,
    ordered: bool,
    start: i64,
    same: bool,
) {
    let kind = if ordered { "orderedList" } else { "bulletList" };
    if depth > 0 {
        if let Some(last) = out
            .last_mut()
            .filter(|v| ["orderedList", "bulletList"].contains(&sval(v, "type")))
        {
            if let Some(item) = last["content"].as_array_mut().and_then(|v| v.last_mut()) {
                if let Some(children) = item["content"].as_array_mut() {
                    append_list(children, blocks, depth - 1, ordered, start, true);
                    return;
                }
            }
        }
    }
    if !same || out.last().map(|v| sval(v, "type")) != Some(kind) {
        out.push(json!({"type":kind,"attrs":{"start":start},"content":[]}));
    }
    out.last_mut().unwrap()["content"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"listItem","content":blocks}));
}

pub fn import(bytes: &[u8]) -> Result<Imported> {
    if bytes.len() as u64 > MAX_TOTAL {
        bail!("文件超过 128 MB，请拆分文档");
    }
    let mut zip = ZipArchive::new(Cursor::new(bytes))
        .context("不是有效的 .docx 文件，旧版 .doc 请先另存为 .docx")?;
    if zip.len() > 2000 {
        bail!("压缩包文件数过多");
    }
    let mut entries = HashMap::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let f = zip.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        if f.size() > MAX_ENTRY {
            bail!("文档内部文件超过 32 MB");
        }
        total += f.size();
        if total > MAX_TOTAL {
            bail!("解压后的文档超过 128 MB");
        }
        let name = f.name().to_string();
        let mut data = vec![];
        f.take(MAX_ENTRY + 1).read_to_end(&mut data)?;
        if data.len() as u64 > MAX_ENTRY {
            bail!("文档内部文件超过限制");
        }
        entries.insert(name, data);
    }
    let raw = entries
        .get("word/document.xml")
        .context("缺少 Word 正文 document.xml")?
        .clone();
    if let Some(snapshot) = entries
        .get("word/rustword.json")
        .and_then(|b| serde_json::from_slice::<Value>(b).ok())
    {
        let unchanged = snapshot["parts"].as_object().is_some_and(|parts| {
            parts.len() + 1 == entries.len()
                && parts.iter().all(|(name, hash)| {
                    entries
                        .get(name)
                        .is_some_and(|bytes| hash.as_str() == Some(&digest(bytes)))
                })
        });
        if unchanged && snapshot["version"] == 2 {
            let doc = snapshot["document"].clone();
            validate(&doc)?;
            return Ok(Imported {
                document: doc,
                warnings: vec![],
            });
        }
    }
    let mut parser = Parser {
        entries,
        rels: HashMap::new(),
        numbering: HashMap::new(),
        warnings: vec![],
    };
    if let Some(raw) = parser.entries.get("word/_rels/document.xml.rels") {
        let d = Document::parse(std::str::from_utf8(raw)?)?;
        for n in d.root_element().children().filter(Node::is_element) {
            if let (Some(id), Some(target)) = (n.attribute("Id"), n.attribute("Target")) {
                let external = n.attribute("TargetMode") == Some("External");
                let target = if external {
                    target.into()
                } else if target.starts_with('/') {
                    target.trim_start_matches('/').into()
                } else {
                    format!("word/{}", target.trim_start_matches("./"))
                };
                parser.rels.insert(id.into(), (target, external));
            }
        }
    }
    if let Some(raw) = parser.entries.get("word/numbering.xml") {
        let d = Document::parse(std::str::from_utf8(raw)?)?;
        let mut defs = HashMap::new();
        for n in d
            .descendants()
            .filter(|n| n.has_tag_name((W, "abstractNum")))
        {
            let id = attr(n, "abstractNumId").unwrap_or("");
            for lvl in n.children().filter(|n| n.has_tag_name((W, "lvl"))) {
                let depth = attr(lvl, "ilvl")
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0);
                let ordered = child(lvl, "numFmt").and_then(|n| attr(n, "val")) != Some("bullet");
                let start = child(lvl, "start")
                    .and_then(|n| attr(n, "val"))
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(1);
                defs.insert((id.to_string(), depth), (ordered, start));
            }
        }
        for n in d.descendants().filter(|n| n.has_tag_name((W, "num"))) {
            let id = attr(n, "numId").unwrap_or("");
            let abstract_id = child(n, "abstractNumId")
                .and_then(|n| attr(n, "val"))
                .unwrap_or("");
            for depth in 0..=8 {
                if let Some(v) = defs.get(&(abstract_id.into(), depth)) {
                    parser.numbering.insert((id.into(), depth), *v);
                }
            }
        }
    }
    let mut styles: Styles = HashMap::new();
    // OOXML without paragraph spacing has zero paragraph margins. New Rust Word
    // documents use 8 pt after by default, but importing must retain the source.
    let mut defaults = Style {
        paragraph: serde_json::from_value(json!({"spaceBefore":0.,"spaceAfter":0.,
            "firstLineIndent":0.,"keepNext":false,"keepLines":false,"pageBreakBefore":false}))?,
        ..Default::default()
    };
    if let Some(raw) = parser.entries.get("word/styles.xml").cloned() {
        let d = Document::parse(std::str::from_utf8(&raw)?)?;
        if let Some(pr) = d
            .descendants()
            .find(|n| n.has_tag_name((W, "rPrDefault")))
            .and_then(|n| child(n, "rPr"))
        {
            defaults.marks = parser.props(pr);
        }
        if let Some(pr) = d
            .descendants()
            .find(|n| n.has_tag_name((W, "pPrDefault")))
            .and_then(|n| child(n, "pPr"))
        {
            defaults.paragraph.extend(parser.paragraph_props(pr));
        }
        for n in d.descendants().filter(|n| n.has_tag_name((W, "style"))) {
            let id = attr(n, "styleId").unwrap_or("");
            let ms = child(n, "rPr").map(|r| parser.props(r)).unwrap_or_default();
            let outline = child(n, "pPr")
                .and_then(|p| child(p, "outlineLvl"))
                .and_then(|n| attr(n, "val"))
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|n| *n < 6)
                .map(|n| n + 1);
            let list = child(n, "pPr")
                .and_then(|n| child(n, "numPr"))
                .and_then(|n| {
                    let id = child(n, "numId").and_then(|n| attr(n, "val"))?.to_string();
                    let depth = child(n, "ilvl")
                        .and_then(|n| attr(n, "val"))
                        .and_then(|v| v.parse::<i64>().ok())
                        .unwrap_or(0);
                    Some((id, depth))
                });
            let parent = child(n, "basedOn")
                .and_then(|n| attr(n, "val"))
                .map(str::to_string);
            let warnings_start = parser.warnings.len();
            let paragraph = child(n, "pPr")
                .map(|pr| parser.paragraph_props(pr))
                .unwrap_or_default();
            // Word bundles many unused built-in styles. Only warn if an affected style
            // (or one of its descendants) is actually used in the document.
            let warnings = parser.warnings.split_off(warnings_start);
            styles.insert(
                id.to_string(),
                Style {
                    marks: ms,
                    paragraph,
                    warnings,
                    outline,
                    list,
                    parent,
                },
            );
        }
    }
    let definitions = styles.clone();
    for id in definitions.keys() {
        styles.insert(id.clone(), resolve_style(id, &definitions, &defaults, 0));
    }
    styles.entry("Normal".into()).or_insert(defaults);
    if parser
        .entries
        .keys()
        .any(|k| k.starts_with("word/header") || k.starts_with("word/footer"))
    {
        parser.warn("页眉、页脚不会显示或保存；请另存副本以保留原文件。");
    }
    if parser
        .entries
        .keys()
        .any(|k| k.contains("comments") || k.contains("footnotes") || k.contains("endnotes"))
    {
        parser.warn("批注、脚注或尾注暂不支持，原文件保留。");
    }
    let d = Document::parse(std::str::from_utf8(&raw)?).context("Word 正文 XML 损坏")?;
    let sections: Vec<_> = d
        .descendants()
        .filter(|n| n.has_tag_name((W, "sectPr")))
        .collect();
    if sections.len() > 1 {
        parser.warn("文档包含多节页面设置，编辑时统一为一节。");
    }
    if sections
        .iter()
        .any(|n| !n.parent().is_some_and(|p| p.has_tag_name((W, "body"))))
    {
        parser.warn("段落内的分节符暂不保留，另存副本以保留原文件。");
    }
    if d.descendants().any(|n| n.tag_name().name() == "anchor") {
        parser.warn("浮动图片转换为正文图片，位置可能变化。");
    }
    let body = d
        .descendants()
        .find(|n| n.has_tag_name((W, "body")))
        .context("找不到 Word 正文")?;
    let mut content = parser.body(body, &styles)?;
    if content.is_empty() {
        content.push(json!({"type":"paragraph"}));
    }
    let section = sections.first().copied();
    let size = section.and_then(|n| child(n, "pgSz"));
    let width = size
        .and_then(|n| attr(n, "w"))
        .and_then(|v| v.parse::<i64>().ok());
    let height = size
        .and_then(|n| attr(n, "h"))
        .and_then(|v| v.parse::<i64>().ok());
    let landscape = size.and_then(|n| attr(n, "orient")) == Some("landscape")
        || width.zip(height).is_some_and(|(w, h)| w > h);
    if let Some((w, h)) = width.zip(height) {
        let (expected_w, expected_h) = if landscape {
            (16838, 11906)
        } else {
            (11906, 16838)
        };
        if (w - expected_w).abs() > 2 || (h - expected_h).abs() > 2 {
            parser.warn("非 A4 纸张编辑时转换为 A4，另存副本以保留原文件。");
        }
    }
    let mut document_attrs = json!({"orientation":if landscape{"landscape"}else{"portrait"},"marginTop":20.,"marginRight":20.,"marginBottom":20.,"marginLeft":20.});
    if let Some(mar) = section.and_then(|n| child(n, "pgMar")) {
        for (xml_key, key) in [
            ("top", "marginTop"),
            ("right", "marginRight"),
            ("bottom", "marginBottom"),
            ("left", "marginLeft"),
        ] {
            if let Some(raw) = attr(mar, xml_key) {
                if let Some(twips) = raw.parse::<f64>().ok().filter(|n| n.is_finite()) {
                    let mm = twips * 25.4 / 1440.;
                    // A rounded boundary (e.g. 5 mm = 283 twips) differs by < 0.01 mm.
                    if !(4.99..=50.01).contains(&mm) {
                        parser.warn("部分页边距超出 5–50 毫米范围，另存副本以保留原文件。");
                    }
                    document_attrs[key] = json!((mm * 100.).round().clamp(500., 5000.) / 100.);
                } else {
                    parser.warn("文档包含无效页边距，已使用默认值。");
                }
            }
        }
        if attr(mar, "gutter").is_some_and(|v| v != "0") {
            parser.warn("装订线页边距暂不支持，另存副本以保留原文件。");
        }
    }
    if let Some(settings) = parser
        .entries
        .get("word/settings.xml")
        .and_then(|raw| std::str::from_utf8(raw).ok())
        .and_then(|raw| Document::parse(raw).ok())
    {
        if settings.descendants().any(|n| {
            n.has_tag_name((W, "mirrorMargins"))
                && !attr(n, "val").is_some_and(|v| ["0", "false", "off"].contains(&v))
        }) {
            parser.warn("镜像页边距暂不支持，另存副本以保留原文件。");
        }
    }
    let document = json!({"type":"doc","attrs":document_attrs,"content":content});
    validate(&document)?;
    Ok(Imported {
        document,
        warnings: parser.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn foreign_parts(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut z = ZipWriter::new(Cursor::new(vec![]));
        for (name, xml) in parts {
            z.start_file(*name, SimpleFileOptions::default()).unwrap();
            z.write_all(xml.as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    }
    fn remove_snapshot(bytes: &[u8]) -> Vec<u8> {
        let mut source = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut output = ZipWriter::new(Cursor::new(vec![]));
        for i in 0..source.len() {
            let mut file = source.by_index(i).unwrap();
            if file.name() != "word/rustword.json" {
                output
                    .start_file(file.name(), SimpleFileOptions::default())
                    .unwrap();
                std::io::copy(&mut file, &mut output).unwrap();
            }
        }
        output.finish().unwrap().into_inner()
    }
    #[test]
    fn rich_docx_roundtrip() {
        let d = json!({"type":"doc","attrs":{"orientation":"landscape"},"content":[{"type":"heading","attrs":{"level":1},"content":[{"type":"text","text":"中文 & Word <编辑器>","marks":[{"type":"bold"}]}]},{"type":"orderedList","attrs":{"start":3},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"内容","marks":[{"type":"textStyle","attrs":{"fontSize":"18pt","color":"#163755"}}]}]}]}]},{"type":"table","content":[{"type":"tableRow","content":[{"type":"tableCell","attrs":{"colspan":1,"rowspan":1},"content":[{"type":"paragraph","content":[{"type":"text","text":"表格"}]}]}]}]}]});
        let bytes = export(&d).unwrap();
        assert_eq!(import(&bytes).unwrap().document, d);
        assert!(bytes.starts_with(b"PK"));
        let mut z = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        z.by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("<w:tbl>"));
        assert!(xml.contains("w:orient=\"landscape\""));
        Document::parse(&xml).unwrap();
    }
    #[test]
    fn foreign_docx_import() {
        let xml=format!("<w:document xmlns:w=\"{W}\"><w:body><w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>外部文档</w:t></w:r></w:p><w:p><w:r><w:t>可以继续编辑</w:t></w:r></w:p></w:body></w:document>");
        let mut z = ZipWriter::new(Cursor::new(vec![]));
        z.start_file("word/document.xml", SimpleFileOptions::default())
            .unwrap();
        z.write_all(xml.as_bytes()).unwrap();
        let d = import(&z.finish().unwrap().into_inner()).unwrap();
        assert_eq!(d.document["content"][0]["type"], "heading");
        assert_eq!(d.document["content"][0]["content"][0]["text"], "外部文档");
        assert_eq!(
            d.document["content"][0]["content"][0]["marks"][0]["type"],
            "bold"
        );
    }
    #[test]
    fn inherited_layout_preserves_explicit_false_without_snapshot() {
        let styles = format!(
            r#"<w:styles xmlns:w="{W}"><w:docDefaults><w:pPrDefault><w:pPr><w:spacing w:after="160"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="Normal"><w:pPr><w:keepNext/><w:keepLines/><w:pageBreakBefore/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Layout"><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="240"/><w:ind w:left="720" w:hanging="360"/></w:pPr></w:style></w:styles>"#
        );
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:pPr><w:pStyle w:val="Layout"/><w:keepNext w:val="false"/><w:keepLines w:val="off"/><w:pageBreakBefore w:val="0"/></w:pPr><w:r><w:t>继承与覆盖</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1134" w:bottom="1701" w:left="850"/></w:sectPr></w:body></w:document>"#
        );
        let imported = import(&foreign_parts(&[
            ("word/document.xml", &xml),
            ("word/styles.xml", &styles),
        ]))
        .unwrap();
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
        let p = &imported.document["content"][0]["attrs"];
        assert_eq!(p["spaceBefore"], 12.);
        assert_eq!(p["spaceAfter"], 8.);
        assert_eq!(p["firstLineIndent"], -18.);
        assert_eq!(p["indent"], 2);
        for key in ["keepNext", "keepLines", "pageBreakBefore"] {
            assert_eq!(p[key], false);
        }
        assert_eq!(imported.document["attrs"]["marginTop"], 25.4);
        let exported = export(&imported.document).unwrap();
        let reopened = import(&remove_snapshot(&exported)).unwrap();
        for key in [
            "spaceBefore",
            "spaceAfter",
            "firstLineIndent",
            "indent",
            "keepNext",
            "keepLines",
            "pageBreakBefore",
        ] {
            assert_eq!(
                reopened.document["content"][0]["attrs"][key], p[key],
                "{key}"
            );
        }
        assert_eq!(reopened.document["attrs"], imported.document["attrs"]);
    }
    #[test]
    fn incompatible_layout_requires_protecting_the_original() {
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:pPr><w:spacing w:line="360" w:lineRule="exact"/><w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:pPr><w:r><w:t>多节</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="-100" w:right="1134" w:bottom="1134" w:left="1134"/></w:sectPr></w:body></w:document>"#
        );
        let imported = import(&foreign_parts(&[("word/document.xml", &xml)])).unwrap();
        assert!(imported.warnings.iter().any(|s| s.contains("多节")));
        assert!(imported.warnings.iter().any(|s| s.contains("分节符")));
        assert!(imported.warnings.iter().any(|s| s.contains("固定值")));
        // An invalid value is recoverable and warning-bearing rather than panicking.
        let invalid = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:pPr><w:spacing w:before="NaN"/><w:ind w:firstLine="invalid"/></w:pPr><w:r><w:t>容错</w:t></w:r></w:p><w:sectPr><w:pgMar w:top="NaN"/></w:sectPr></w:body></w:document>"#
        );
        let imported = import(&foreign_parts(&[("word/document.xml", &invalid)])).unwrap();
        assert!(imported.warnings.iter().any(|s| s.contains("无效段落间距")));
        assert!(imported.warnings.iter().any(|s| s.contains("无效首行缩进")));
        assert!(imported.warnings.iter().any(|s| s.contains("无效页边距")));
        assert_eq!(imported.document["attrs"]["marginTop"], 20.);
    }
    #[test]
    fn rejects_invalid_and_unknown() {
        assert!(import(b"not zip").is_err());
        assert!(export(&json!({"type":"doc","content":[{"type":"magic"}]})).is_err());
    }
}
