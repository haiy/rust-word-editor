mod ai;
mod cloud;
mod docx;
mod drives;
mod i18n;
mod materials;
mod media;
mod ppt;
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

const HTML: &str = include_str!("../dist/index.html");
#[derive(Debug)]
enum UserEvent {
    Ipc(String),
    Menu(String),
    AiReply(Value),
}
struct App {
    path: Option<PathBuf>,
    imported_name: Option<String>,
    protect_original: bool,
    recovery: PathBuf,
    ui_preferences: PathBuf,
    ui_locale: String,
    ui_snapshot: Option<Value>,
    test_root: Option<PathBuf>,
    incoming_path: Option<PathBuf>,
    ai: ai::Service,
    templates: PathBuf,
    storage: cloud::Storage,
    materials: materials::Library,
    drives: drives::Service,
}
impl App {
    fn new(test_root: Option<PathBuf>) -> Self {
        let config_dir = if let Some(root) = &test_root {
            root.clone()
        } else {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                .join("Library/Application Support/Rust Word")
        };
        Self {
            path: None,
            imported_name: None,
            protect_original: false,
            recovery: config_dir.join("recovery.json"),
            ui_locale: fs::read(config_dir.join("ui-preferences.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| {
                    v["locale"]
                        .as_str()
                        .filter(|s| ["en", "zh-CN"].contains(s))
                        .map(str::to_owned)
                })
                .unwrap_or_default(),
            ui_preferences: config_dir.join("ui-preferences.json"),
            ui_snapshot: None,
            test_root,
            incoming_path: None,
            ai: ai::Service::new(&config_dir),
            templates: config_dir.join("templates.json"),
            storage: cloud::Storage::new(&config_dir),
            materials: materials::Library::new(&config_dir),
            drives: drives::Service::new(&config_dir),
        }
    }
    fn load_templates(&self) -> Result<Vec<Value>> {
        let raw = match fs::read(&self.templates) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(_) => bail!("无法读取本机模板库"),
        };
        if raw.len() > 8 * 1024 * 1024 {
            bail!("本机模板库超过 8 MB，请减少模板数量");
        }
        let entries: Vec<Value> = serde_json::from_slice(&raw).context("本机模板库格式损坏")?;
        if entries.len() > 100 {
            bail!("本机模板库最多保存 100 个模板");
        }
        for entry in &entries {
            validate_template(entry)?;
        }
        Ok(entries)
    }
    fn store_templates(&self, entries: &[Value]) -> Result<()> {
        let raw = serde_json::to_vec(entries)?;
        if entries.len() > 100 {
            bail!("本机模板库最多保存 100 个模板");
        }
        if raw.len() > 8 * 1024 * 1024 {
            bail!("本机模板库最多 8 MB，请删除大型模板后重试");
        }
        atomic_write(&self.templates, &raw)
    }
    fn save_template(&self, req: &Value) -> Result<Value> {
        let mut entries = self.load_templates()?;
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
        let now = timestamp.as_millis();
        let id = if let Some(id) = req["id"].as_str() {
            // IPC request IDs are separate; a template ID may optionally be supplied as templateId.
            req["templateId"].as_str().unwrap_or(id).to_owned()
        } else if let Some(id) = req["templateId"].as_str() {
            id.to_owned()
        } else {
            format!("user-{}-{}", timestamp.as_nanos(), std::process::id())
        };
        let existing = entries
            .iter()
            .position(|entry| entry["id"].as_str() == Some(&id));
        let title = req["title"].as_str().context("请输入模板名称")?.trim();
        let entry = json!({"id":id,"title":title,"category":req["category"].as_str().unwrap_or("我的模板").trim(),
            "description":req["description"].as_str().unwrap_or("").trim(),"document":req["document"],
            "createdAt":existing.map(|index|entries[index]["createdAt"].clone()).unwrap_or_else(||json!(now as u64))});
        validate_template(&entry)?;
        if let Some(index) = existing {
            entries[index] = entry.clone();
        } else {
            entries.push(entry.clone());
        }
        self.store_templates(&entries)?;
        Ok(entry)
    }
    fn test_path(&self, path: &str) -> Result<PathBuf> {
        let root = self.test_root.as_ref().context("此操作仅用于隔离测试")?;
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        let parent = path.parent().context("无效路径")?.canonicalize()?;
        if !parent.starts_with(root.canonicalize()?) {
            bail!("路径超出隔离测试目录");
        }
        if path.exists() && !path.canonicalize()?.starts_with(root.canonicalize()?) {
            bail!("文件链接超出隔离测试目录");
        }
        Ok(path)
    }
    fn open(&mut self, path: PathBuf) -> Result<Value> {
        let meta = fs::metadata(&path)?;
        if meta.len() > 128 * 1024 * 1024 {
            bail!("文件超过 128 MB");
        }
        let bytes = fs::read(&path)?;
        let (document, warnings) = parse_local_document(&file_name(&path), &bytes)?;
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        self.imported_name = None;
        self.protect_original = !warnings.is_empty() || ext != "docx";
        self.path = Some(path.clone());
        Ok(
            json!({"document":document,"path":path.to_string_lossy(),"name":file_name(&path),"warnings":warnings,"protected":self.protect_original}),
        )
    }
    fn dispatch(&mut self, req: &Value) -> Result<Value> {
        match req["command"].as_str().unwrap_or("") {
            "uiPreferences" => {
                if self.ui_locale.is_empty() {
                    self.ui_locale = if req["browserLanguage"]
                        .as_str()
                        .unwrap_or("en")
                        .starts_with("zh")
                    {
                        "zh-CN"
                    } else {
                        "en"
                    }
                    .into();
                }
                Ok(json!({"locale":self.ui_locale,"snapshot":self.ui_snapshot.take()}))
            }
            "setUiLanguage" => {
                let locale = req["locale"].as_str().context("Invalid UI language")?;
                if !["en", "zh-CN"].contains(&locale) {
                    bail!("Unsupported UI language");
                }
                docx::validate(&req["snapshot"]["document"])?;
                if serde_json::to_vec(&req["snapshot"])?.len() > 128 * 1024 * 1024 {
                    bail!("Workspace is too large to reload safely");
                }
                atomic_write(
                    &self.ui_preferences,
                    &serde_json::to_vec(&json!({"locale":locale}))?,
                )?;
                self.ui_locale = locale.into();
                self.ui_snapshot = Some(req["snapshot"].clone());
                Ok(json!({"ok":true}))
            }
            "aiSettings" => self.ai.settings(),
            "saveAiSettings" => self.ai.save_settings(&req["settings"]),
            "aiRewrite" => self.ai.rewrite(req),
            "aiTranslate" => self.ai.translate_document(req),
            "aiFormat" => self.ai.format_document(req),
            "aiPresentation" => self.ai.presentation_outline(req),
            "cloudSettings" | "saveCloudSettings" | "cloudList" | "cloudTest" | "cloudUpload"
            | "cloudDownload" => self.storage.dispatch(req["command"].as_str().unwrap(), req),
            "cloudSetIcloud" => self.storage.set_icloud(),
            "cloudSetFolder" | "cloudSetFolderPath" => {
                let path = if req["command"] == "cloudSetFolderPath" {
                    Some(self.test_path(req["path"].as_str().context("缺少路径")?)?)
                } else {
                    rfd::FileDialog::new()
                        .set_title(i18n::text(
                            "选择 iCloud / Nextcloud / 其他同步文件夹",
                            &self.ui_locale,
                        ))
                        .pick_folder()
                };
                match path {
                    Some(p) => self.storage.set_folder(&p, "folder"),
                    None => Ok(Value::Null),
                }
            }
            "materialList" | "materialGet" | "materialSave" | "materialDelete" => self
                .materials
                .dispatch(req["command"].as_str().unwrap(), req),
            "imageInspect" | "imageEdit" | "pdfInspect" | "pdfRender" | "pdfToText"
            | "pdfTransform" | "pdfMerge" | "imagesToPdf" => media::dispatch(
                req["command"].as_str().unwrap(),
                req,
                self.test_root.as_deref(),
            ),
            "driveDiscover" | "driveConnect" | "driveList" | "driveUpload" | "driveDownload"
            | "driveDisconnect" => self.drives.dispatch(req["command"].as_str().unwrap(), req),
            "pptTemplateOpen" => {
                ppt::template_open(req["templateId"].as_str().context("缺少模板 ID")?)
            }
            "pptTemplateFile" => ppt::template_file(
                req["templateId"].as_str().context("缺少模板 ID")?,
                req["original"].as_bool().unwrap_or(false),
            ),
            "pptTemplatePreview" => {
                ppt::template_preview(req["templateId"].as_str().context("缺少模板 ID")?)
            }
            "pptNativeRender" => ppt::render_native(req),
            "pptThemes" => ppt::themes(),
            "pptImport" => {
                let bytes = cloud::decode(req["data"].as_str().context("缺少演示文稿内容")?)?;
                ppt::import_own_pptx(&bytes)
            }
            "pptExport" => {
                let bytes = ppt::export_pptx(&req["deck"])?;
                Ok(
                    json!({"data":STANDARD.encode(&bytes),"name":cloud::suggested_name(req["deck"]["title"].as_str().unwrap_or("演示文稿"),"pptx"),"bytes":bytes.len()}),
                )
            }
            "pptExportImages" => {
                let bytes = ppt::export_images(req)?;
                Ok(
                    json!({"data":STANDARD.encode(&bytes),"name":"演示图片.zip","bytes":bytes.len()}),
                )
            }
            "exportDocument" => {
                let bytes = docx::export(&req["document"])?;
                Ok(
                    json!({"data":STANDARD.encode(&bytes),"bytes":bytes.len(),"name":cloud::suggested_name(req["name"].as_str().unwrap_or("文档"),"docx")}),
                )
            }
            "openFile" => {
                let name = req["name"].as_str().context("缺少文件名")?;
                if name.is_empty()
                    || name
                        .chars()
                        .any(|c| c == '/' || c == '\\' || c.is_control())
                {
                    bail!("文件名无效");
                }
                let bytes = cloud::decode(req["data"].as_str().context("缺少文档内容")?)?;
                let (document, warnings) = parse_local_document(name, &bytes)?;
                // Uploaded/dropped files have no writable local path. Never retain
                // the previous document's save target when opening their contents.
                self.path = None;
                self.imported_name = Some(name.to_owned());
                self.protect_original = true;
                Ok(json!({"document":document,"warnings":warnings,"name":name,"protected":true}))
            }
            "importDocument" => {
                let bytes = cloud::decode(req["data"].as_str().context("缺少文档内容")?)?;
                let imported = docx::import(&bytes)?;
                Ok(
                    json!({"document":imported.document,"warnings":imported.warnings,"name":req["name"].as_str().unwrap_or("云文档副本")}),
                )
            }
            "pickBytes" | "pickBytesPath" => {
                let kind = req["kind"].as_str().unwrap_or("file");
                let path = if req["command"] == "pickBytesPath" {
                    Some(self.test_path(req["path"].as_str().context("缺少路径")?)?)
                } else {
                    let mut d =
                        rfd::FileDialog::new().set_title(i18n::text("选择文件", &self.ui_locale));
                    d = match kind {
                        "pdf" => d.add_filter("PDF", &["pdf"]),
                        "ppt" => d.add_filter("PPTX", &["pptx"]),
                        "image" => d.add_filter(
                            "图片",
                            &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff"],
                        ),
                        _ => d.add_filter(
                            "办公文件",
                            &[
                                "docx", "pptx", "potx", "pdf", "png", "jpg", "jpeg", "webp", "txt",
                            ],
                        ),
                    };
                    d.pick_file()
                };
                let Some(p) = path else {
                    return Ok(Value::Null);
                };
                let meta = fs::metadata(&p)?;
                if meta.len() > 96 * 1024 * 1024 {
                    bail!("文件超过 96 MB")
                };
                if !meta.is_file() {
                    bail!("请选择一个文件")
                };
                let data = STANDARD.encode(fs::read(&p)?);
                Ok(json!({"data":data,"name":file_name(&p),"bytes":meta.len()}))
            }
            "saveBytes" | "saveBytesPath" => {
                let bytes = cloud::decode(req["data"].as_str().context("缺少导出内容")?)?;
                let name = req["name"].as_str().unwrap_or("导出文件.bin");
                let clean = Path::new(name)
                    .file_name()
                    .context("导出名称无效")?
                    .to_string_lossy()
                    .to_string();
                let path = if req["command"] == "saveBytesPath" {
                    Some(self.test_path(req["path"].as_str().context("缺少路径")?)?)
                } else {
                    rfd::FileDialog::new()
                        .set_title(i18n::text("保存导出文件", &self.ui_locale))
                        .set_file_name(&clean)
                        .save_file()
                };
                match path {
                    Some(p) => {
                        atomic_write(&p, &bytes)?;
                        Ok(
                            json!({"path":p.to_string_lossy(),"name":file_name(&p),"bytes":bytes.len()}),
                        )
                    }
                    None => Ok(Value::Null),
                }
            }
            "listTemplates" => Ok(json!({"templates":self.load_templates()?})),
            "saveTemplate" => self.save_template(req),
            "deleteTemplate" => {
                let id = req["templateId"]
                    .as_str()
                    .or_else(|| req["id"].as_str())
                    .context("缺少模板 ID")?;
                validate_template_id(id)?;
                let mut entries = self.load_templates()?;
                let original = entries.len();
                entries.retain(|entry| entry["id"].as_str() != Some(id));
                if entries.len() == original {
                    bail!("未找到该本机模板");
                }
                self.store_templates(&entries)?;
                Ok(json!({"ok":true}))
            }
            "ready" => {
                let recovery = fs::read(&self.recovery)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .filter(|v| docx::validate(&v["document"]).is_ok());
                Ok(
                    json!({"recovery":recovery,"hasPendingOpen":self.incoming_path.is_some(),"version":env!("CARGO_PKG_VERSION")}),
                )
            }
            "new" => {
                self.path = None;
                self.imported_name = None;
                self.protect_original = false;
                let _ = fs::remove_file(&self.recovery);
                Ok(json!({"ok":true}))
            }
            "open" => {
                let path = rfd::FileDialog::new()
                    .set_title(i18n::text("打开文档", &self.ui_locale))
                    .add_filter(i18n::text("Word / 文本", &self.ui_locale), &["docx", "txt"])
                    .pick_file();
                if let Some(p) = path {
                    self.open(p)
                } else {
                    Ok(Value::Null)
                }
            }
            "openPath" => {
                let p = self.test_path(req["path"].as_str().context("缺少路径")?)?;
                self.open(p)
            }
            "openPending" => {
                if let Some(p) = self.incoming_path.take() {
                    self.open(p)
                } else {
                    Ok(Value::Null)
                }
            }
            "save" | "saveAs" | "savePath" => {
                let document = &req["document"];
                docx::validate(document)?;
                let command = req["command"].as_str().unwrap();
                let path = if command == "savePath" {
                    Some(self.test_path(req["path"].as_str().context("缺少路径")?)?)
                } else if command == "save" && self.path.is_some() && !self.protect_original {
                    self.path.clone()
                } else {
                    let name = self
                        .path
                        .as_deref()
                        .or_else(|| self.imported_name.as_deref().map(Path::new))
                        .and_then(|p| p.file_stem())
                        .and_then(|s| s.to_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| i18n::text("未命名文档", &self.ui_locale));
                    rfd::FileDialog::new()
                        .set_title(i18n::text(
                            if self.protect_original {
                                "另存副本，保留原件"
                            } else {
                                "保存 Word 文档"
                            },
                            &self.ui_locale,
                        ))
                        .add_filter(i18n::text("Word 文档", &self.ui_locale), &["docx"])
                        .set_file_name(format!(
                            "{name}{}.docx",
                            if self.protect_original {
                                i18n::text("-编辑副本", &self.ui_locale)
                            } else {
                                String::new()
                            }
                        ))
                        .save_file()
                };
                let Some(mut path) = path else {
                    return Ok(Value::Null);
                };
                if path.extension().and_then(|s| s.to_str()) != Some("docx") {
                    path.set_extension("docx");
                }
                if self.protect_original && self.path.as_ref() == Some(&path) {
                    bail!("此文档包含暂不支持的内容，请使用不同文件名保存副本以保护原件");
                }
                let bytes = docx::export(document)?;
                atomic_write(&path, &bytes)?;
                self.path = Some(path.clone());
                self.imported_name = None;
                self.protect_original = false;
                let _ = fs::remove_file(&self.recovery);
                Ok(
                    json!({"path":path.to_string_lossy(),"name":file_name(&path),"bytes":bytes.len()}),
                )
            }
            "autosave" => {
                docx::validate(&req["document"])?;
                let data = serde_json::to_vec(
                    &json!({"document":req["document"],"source":self.path.as_ref().map(|p|p.to_string_lossy().to_string()),"name":self.path.as_ref().map(|p|file_name(p)).unwrap_or_else(||"未命名文档".into()),"savedAt":req["savedAt"]}),
                )?;
                atomic_write(&self.recovery, &data)?;
                Ok(json!({"ok":true}))
            }
            "discardRecovery" => {
                match fs::remove_file(&self.recovery) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                };
                Ok(json!({"ok":true}))
            }
            "recover" => {
                let raw = fs::read(&self.recovery)?;
                let mut v: Value = serde_json::from_slice(&raw)?;
                docx::validate(&v["document"])?;
                // A recovered draft is always a new copy, never silently linked to the source.
                self.path = None;
                self.protect_original = false;
                self.imported_name = None;
                v["path"] = Value::Null;
                Ok(v)
            }
            "image" => {
                let path = rfd::FileDialog::new()
                    .set_title(i18n::text("插入图片", &self.ui_locale))
                    .add_filter(
                        i18n::text("图片", &self.ui_locale),
                        &["png", "jpg", "jpeg", "gif"],
                    )
                    .pick_file();
                if let Some(p) = path {
                    image_file(&p)
                } else {
                    Ok(Value::Null)
                }
            }
            "imagePath" => {
                let p = self.test_path(req["path"].as_str().context("缺少路径")?)?;
                image_file(&p)
            }
            _ => bail!("未知命令"),
        }
    }
}
fn validate_template_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        bail!("模板 ID 格式无效");
    }
    Ok(())
}
fn validate_template(entry: &Value) -> Result<()> {
    validate_template_id(entry["id"].as_str().context("缺少模板 ID")?)?;
    let title = entry["title"].as_str().context("缺少模板名称")?;
    if title.trim().is_empty() || title.chars().count() > 80 {
        bail!("模板名称需为 1 至 80 字");
    }
    if entry["category"]
        .as_str()
        .context("模板分类无效")?
        .chars()
        .count()
        > 40
    {
        bail!("模板分类最多 40 字");
    }
    if entry["description"]
        .as_str()
        .context("模板简介无效")?
        .chars()
        .count()
        > 500
    {
        bail!("模板简介最多 500 字");
    }
    docx::validate(&entry["document"])?;
    Ok(())
}
fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "未命名文档".into())
}
fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().context("无效保存位置")?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(data)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .map_err(|e| e.error)
        .context("保存失败，原文件未被修改")?;
    Ok(())
}
fn image_file(p: &Path) -> Result<Value> {
    let meta = fs::metadata(p)?;
    if meta.len() > 32 * 1024 * 1024 {
        bail!("图片超过 32 MB");
    }
    let ext = p
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        _ => bail!("请选择 PNG/JPEG/GIF 图片"),
    };
    Ok(
        json!({"src":format!("data:{mime};base64,{}",STANDARD.encode(fs::read(p)?)),"alt":file_name(p)}),
    )
}
fn parse_local_document(name: &str, bytes: &[u8]) -> Result<(Value, Vec<String>)> {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let (document, warnings) = if ext == "txt" {
        let s = String::from_utf8(bytes.to_vec()).context("文本文件需为 UTF-8 编码")?;
        (
            json!({"type":"doc","content":s.lines().map(|line|if line.is_empty(){json!({"type":"paragraph"})}else{json!({"type":"paragraph","content":[{"type":"text","text":line}]})}).collect::<Vec<_>>()}),
            vec![],
        )
    } else if ext == "docx" {
        let d = docx::import(bytes)?;
        (d.document, d.warnings)
    } else {
        bail!("请选择 .docx 或 UTF-8 .txt 文件，旧版 .doc 请先转换");
    };
    Ok((document, warnings))
}

fn response(app: &mut App, req: &Value) -> Value {
    let id = req["id"].clone();
    match app.dispatch(req) {
        Ok(data) => json!({"id":id,"data":data}),
        Err(e) => json!({"id":id,"error":format!("{e:#}")}),
    }
}

fn rpc(root: &str) -> Result<()> {
    fs::create_dir_all(root)?;
    let mut app = App::new(Some(PathBuf::from(root)));
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        match serde_json::from_str::<Value>(&line) {
            Ok(req) => println!("{}", response(&mut app, &req)),
            Err(e) => println!("{}", json!({"error":e.to_string()})),
        };
    }
    Ok(())
}
fn test_server(addr: &str, root: &str) -> Result<()> {
    fs::create_dir_all(root)?;
    let mut app = App::new(Some(PathBuf::from(root)));
    let server = tiny_http::Server::http(addr).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("TEST_SERVER http://{addr}，隔离目录 {root}");
    for mut request in server.incoming_requests() {
        let path = request.url().split('?').next().unwrap_or("");
        let (body, mime) = if request.method() == &tiny_http::Method::Get && path == "/" {
            (HTML.as_bytes().to_vec(), "text/html; charset=utf-8")
        } else if request.method() == &tiny_http::Method::Post && path == "/rpc" {
            let mut b = String::new();
            request
                .as_reader()
                .take(150 * 1024 * 1024)
                .read_to_string(&mut b)?;
            let value = match serde_json::from_str::<Value>(&b) {
                Ok(req) => response(&mut app, &req),
                Err(e) => json!({"error":e.to_string()}),
            };
            (serde_json::to_vec(&value)?, "application/json")
        } else {
            (b"Not found".to_vec(), "text/plain")
        };
        request.respond(
            tiny_http::Response::from_data(body)
                .with_header(tiny_http::Header::from_bytes("Content-Type", mime).unwrap()),
        )?;
    }
    Ok(())
}
fn native_menu(locale: &str) -> Result<muda::Menu> {
    use muda::{accelerator::Accelerator, Menu, MenuItem, PredefinedMenuItem, Submenu};
    let item = |id: &str, label: &str, key: &str| {
        MenuItem::with_id(id, label, true, key.parse::<Accelerator>().ok())
    };
    let menu = Menu::new();
    let app = Submenu::new("Rust Word", true);
    app.append_items(&[
        &PredefinedMenuItem::about(
            Some(&i18n::text("关于 Rust Word", locale)),
            Some(muda::AboutMetadata {
                name: Some("Rust Word".into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                ..Default::default()
            }),
        ),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(Some(&i18n::text("隐藏 Rust Word", locale))),
        &PredefinedMenuItem::hide_others(Some(&i18n::text("隐藏其他", locale))),
        &PredefinedMenuItem::show_all(Some(&i18n::text("显示全部", locale))),
        &PredefinedMenuItem::separator(),
        &item("quit", &i18n::text("退出 Rust Word", locale), "Cmd+Q"),
    ])?;
    let file = Submenu::new(i18n::text("文件", locale), true);
    file.append_items(&[
        &item("new", &i18n::text("新建", locale), "Cmd+N"),
        &item("open", &i18n::text("打开…", locale), "Cmd+O"),
        &PredefinedMenuItem::separator(),
        &item("save", &i18n::text("保存", locale), "Cmd+S"),
        &item("saveAs", &i18n::text("另存为…", locale), "Cmd+Shift+S"),
        &PredefinedMenuItem::separator(),
        &item("print", &i18n::text("打印 / PDF…", locale), "Cmd+P"),
        &item("quit", &i18n::text("关闭窗口", locale), "Cmd+W"),
    ])?;
    let edit = Submenu::new(i18n::text("编辑", locale), true);
    edit.append_items(&[
        &item("undo", &i18n::text("撤销", locale), "Cmd+Z"),
        &item("redo", &i18n::text("重做", locale), "Cmd+Shift+Z"),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::cut(Some(&i18n::text("剪切", locale))),
        &PredefinedMenuItem::copy(Some(&i18n::text("复制", locale))),
        &PredefinedMenuItem::paste(Some(&i18n::text("粘贴", locale))),
        &PredefinedMenuItem::paste_and_match_style(Some(&i18n::text("粘贴并匹配样式", locale))),
        &PredefinedMenuItem::select_all(Some(&i18n::text("全选", locale))),
        &PredefinedMenuItem::separator(),
        &item("find", &i18n::text("查找与替换", locale), "Cmd+F"),
    ])?;
    let tools = Submenu::new(i18n::text("写作", locale), true);
    tools.append_items(&[
        &MenuItem::with_id("templates", i18n::text("模板库…", locale), true, None),
        &MenuItem::with_id("ai", i18n::text("AI 辅助修改…", locale), true, None),
        &MenuItem::with_id("aiSettings", i18n::text("AI 设置…", locale), true, None),
        &MenuItem::with_id(
            "translateDocument",
            i18n::text("全文翻译…", locale),
            true,
            None,
        ),
        &MenuItem::with_id("formatDocument", i18n::text("AI 排版…", locale), true, None),
        &MenuItem::with_id(
            "pdfTools",
            i18n::text("PDF 编辑与转换…", locale),
            true,
            None,
        ),
        &MenuItem::with_id("imageTools", i18n::text("图片编辑…", locale), true, None),
        &MenuItem::with_id("cloud", i18n::text("个人云空间…", locale), true, None),
        &MenuItem::with_id("materials", i18n::text("素材库…", locale), true, None),
        &MenuItem::with_id("pptTemplates", i18n::text("PPT 模板…", locale), true, None),
    ])?;
    menu.append_items(&[&app, &file, &edit, &tools])?;
    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();
    Ok(menu)
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--rpc") {
        return rpc(args.get(2).context("需要隔离目录")?);
    }
    if args.get(1).map(String::as_str) == Some("--test-server") {
        return test_server(
            args.get(2).context("需要监听地址")?,
            args.get(3).context("需要隔离目录")?,
        );
    }
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Rust Word")
        .with_inner_size(LogicalSize::new(1260., 900.))
        .with_min_inner_size(LogicalSize::new(860., 600.))
        .build(&event_loop)?;
    let mut menu = native_menu("en")?;
    let menu_proxy = event_loop.create_proxy();
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event.id.0));
    }));
    let proxy = event_loop.create_proxy();
    let webview = WebViewBuilder::new()
        .with_html(HTML)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(UserEvent::Ipc(req.body().clone()));
        })
        .with_navigation_handler(|url| url == "about:blank")
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .build(&window)?;
    let test_root = if args.get(1).map(String::as_str) == Some("--test-root") {
        Some(PathBuf::from(args.get(2).context("需要隔离测试目录")?))
    } else {
        std::env::var_os("RUST_WORD_TEST_ROOT").map(PathBuf::from)
    };
    if let Some(root) = &test_root {
        fs::create_dir_all(root)?;
    }
    let mut app = App::new(test_root);
    let ai_proxy = event_loop.create_proxy();
    app.incoming_path = args
        .get(1)
        .filter(|s| !s.starts_with('-'))
        .map(PathBuf::from);
    event_loop.run(move |event, _, control_flow| {
        let _=&menu;
        *control_flow = ControlFlow::Wait;
        match event {
            Event::Opened{urls}=>{
                if let Some(path)=urls.first().and_then(|url|url.to_file_path().ok()){
                    app.incoming_path=Some(path);
                    let _=webview.evaluate_script("window.rustWord?.requestIncomingOpen()");
                }
            }
            Event::UserEvent(UserEvent::Menu(id))=>{
                let script=if id=="quit"{"window.rustWord?.requestClose()".into()}else{format!("Promise.resolve().then(()=>window.rustWord?.actions[{}]?.()).catch(e=>window.rustWord?.toast(e.message))",json!(id))};
                let _=webview.evaluate_script(&script);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                let _ = webview.evaluate_script("window.rustWord?.requestClose()");
            }
            Event::UserEvent(UserEvent::Ipc(raw)) => {
                if let Ok(req) = serde_json::from_str::<Value>(&raw) {
                    let command = req["command"].as_str().unwrap_or("");
                    if command == "reloadUi" {
                        let _ = webview.load_html(HTML);
                        return;
                    }
                    if command == "exit" {
                        app.ai.shutdown();
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                    if command == "print" {
                        if let Err(e) = webview.print() {
                            let result = json!({"id":req["id"],"error":e.to_string()});
                            let _ =
                                webview.evaluate_script(&format!("window.__rustReply({result})"));
                        } else {
                            let _ = webview.evaluate_script(&format!(
                                "window.__rustReply({})",
                                json!({"id":req["id"],"data":{"ok":true}})
                            ));
                        }
                        return;
                    }
                    if command == "title" {
                        window.set_title(req["title"].as_str().unwrap_or("Rust Word"));
                        return;
                    }
                    if matches!(command,"aiRewrite"|"aiTranslate"|"aiFormat"|"aiPresentation"|"cloudList"|"cloudTest"|"cloudUpload"|"cloudDownload"|"materialGet"|"materialSave"|"imageInspect"|"imageEdit"|"pdfInspect"|"pdfRender"|"pdfToText"|"pdfTransform"|"pdfMerge"|"imagesToPdf"|"pptExport"|"pptExportImages"|"pptTemplateOpen"|"pptTemplateFile"|"pptTemplatePreview"|"pptNativeRender"|"driveDiscover"|"driveConnect"|"driveList"|"driveUpload"|"driveDownload") {
                        let ai=app.ai.clone(); let storage=app.storage.clone();let materials=app.materials.clone();let drives=app.drives.clone();let root=app.test_root.clone();let proxy=ai_proxy.clone();
                        std::thread::spawn(move || {
                            let id=req["id"].clone();let action=req["command"].as_str().unwrap_or("");
                            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->Result<Value>{match action{
                                "driveDiscover"|"driveConnect"|"driveList"|"driveUpload"|"driveDownload"=>drives.dispatch(action,&req),
                                "pptTemplateOpen"=>ppt::template_open(req["templateId"].as_str().context("缺少模板 ID")?),
                                "pptTemplateFile"=>ppt::template_file(req["templateId"].as_str().context("缺少模板 ID")?,req["original"].as_bool().unwrap_or(false)),
                                "pptTemplatePreview"=>ppt::template_preview(req["templateId"].as_str().context("缺少模板 ID")?),
                                "pptNativeRender"=>ppt::render_native(&req),
                                "aiRewrite"=>ai.rewrite(&req),"aiTranslate"=>ai.translate_document(&req),"aiFormat"=>ai.format_document(&req),"aiPresentation"=>ai.presentation_outline(&req),
                                "cloudList"|"cloudTest"|"cloudUpload"|"cloudDownload"=>storage.dispatch(action,&req),
                                "materialGet"|"materialSave"=>materials.dispatch(action,&req),
                                "pptExport"=>{let bytes=ppt::export_pptx(&req["deck"])?;Ok(json!({"data":STANDARD.encode(&bytes),"bytes":bytes.len(),"name":cloud::suggested_name(req["deck"]["title"].as_str().unwrap_or("演示文稿"),"pptx")}))},
                                "pptExportImages"=>{let bytes=ppt::export_images(&req)?;Ok(json!({"data":STANDARD.encode(&bytes),"bytes":bytes.len(),"name":"演示图片.zip"}))},
                                _=>media::dispatch(action,&req,root.as_deref()),
                            }}));
                            let reply=match result{Ok(Ok(data))=>json!({"id":id,"data":data}),Ok(Err(error))=>json!({"id":id,"error":format!("{error:#}")}),Err(_)=>json!({"id":id,"error":"操作异常，请重试"})};let _=proxy.send_event(UserEvent::AiReply(reply));
                        });return;
                    }
                    let reply = response(&mut app, &req);
                    if matches!(command, "uiPreferences" | "setUiLanguage") {
                        if let Ok(updated) = native_menu(&app.ui_locale) { menu = updated; }
                    }
                    let _ = webview.evaluate_script(&format!("window.__rustReply({reply})"));
                }
            }
            Event::UserEvent(UserEvent::AiReply(reply)) => {
                let _ = webview.evaluate_script(&format!("window.__rustReply({reply})"));
            }
            Event::LoopDestroyed => app.ai.shutdown(),
            _ => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_persist_and_validate_before_modifying() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(Some(root.path().into()));
        let doc = json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"可编辑模板正文"}]}]});
        let entry = app
            .dispatch(&json!({"command":"saveTemplate","title":" 我的报告 ","document":doc}))
            .unwrap();
        assert_eq!(entry["title"], "我的报告");
        let id = entry["id"].as_str().unwrap();
        assert!(id.starts_with("user-"));
        let mut reopened = App::new(Some(root.path().into()));
        assert_eq!(
            reopened
                .dispatch(&json!({"command":"listTemplates"}))
                .unwrap()["templates"][0],
            entry
        );
        let before = fs::read(&app.templates).unwrap();
        assert!(app
            .dispatch(&json!({"command":"saveTemplate","title":"字".repeat(81),"document":doc}))
            .is_err());
        assert!(app.dispatch(&json!({"command":"saveTemplate","title":"不支持内容","document":{"type":"doc","content":[{"type":"unsafe"}]}})).is_err());
        assert!(app
            .dispatch(&json!({"command":"deleteTemplate","templateId":"../../outside"}))
            .is_err());
        assert_eq!(fs::read(&app.templates).unwrap(), before);
        let updated = app.dispatch(&json!({"command":"saveTemplate","templateId":id,"title":"更新后的报告","document":doc})).unwrap();
        assert_eq!(updated["createdAt"], entry["createdAt"]);
        assert_eq!(app.load_templates().unwrap().len(), 1);
        app.dispatch(&json!({"command":"deleteTemplate","templateId":id}))
            .unwrap();
        assert_eq!(app.load_templates().unwrap().len(), 0);
        assert!(app
            .dispatch(&json!({"command":"deleteTemplate","templateId":id}))
            .is_err());
    }
    #[test]
    fn template_library_limit_preserves_previous_file() {
        let root = tempfile::tempdir().unwrap();
        let app = App::new(Some(root.path().into()));
        let doc = json!({"type":"doc","content":[{"type":"paragraph"}]});
        let entries = (0..100).map(|i|json!({"id":format!("user-{i}"),"title":"模板","category":"我的模板","description":"","document":doc,"createdAt":0})).collect::<Vec<_>>();
        app.store_templates(&entries).unwrap();
        let before = fs::read(&app.templates).unwrap();
        let mut over_limit = entries.clone();
        over_limit.push(entries[0].clone());
        assert!(app.store_templates(&over_limit).is_err());
        assert_eq!(fs::read(&app.templates).unwrap(), before);
        let mut too_large = entries;
        too_large[0]["document"] = json!("字".repeat(3 * 1024 * 1024));
        assert!(app.store_templates(&too_large).is_err());
        assert_eq!(fs::read(&app.templates).unwrap(), before);
    }
    #[test]
    fn local_upload_detaches_previous_save_target_and_failure_preserves_it() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(Some(root.path().into()));
        let old_doc = json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Original file"}]}]});
        app.dispatch(&json!({"command":"savePath","path":"original.docx","document":old_doc}))
            .unwrap();
        let original = fs::read(root.path().join("original.docx")).unwrap();
        let old_path = app.path.clone();
        assert!(app.dispatch(&json!({"command":"openFile","name":"broken.docx","data":STANDARD.encode(b"not a zip")})).is_err());
        assert_eq!(app.path, old_path);
        assert!(!app.protect_original);
        let new_doc = json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Imported content"}]}]});
        let bytes = docx::export(&new_doc).unwrap();
        let opened = app
            .dispatch(
                &json!({"command":"openFile","name":"report.DOCX","data":STANDARD.encode(bytes)}),
            )
            .unwrap();
        assert_eq!(opened["document"], new_doc);
        assert!(app.path.is_none());
        assert!(app.protect_original);
        assert_eq!(app.imported_name.as_deref(), Some("report.DOCX"));
        assert!(app
            .dispatch(&json!({"command":"openFile","name":"broken.docx","data":"invalid"}))
            .is_err());
        assert_eq!(app.imported_name.as_deref(), Some("report.DOCX"));
        app.dispatch(&json!({"command":"savePath","path":"copy.docx","document":new_doc}))
            .unwrap();
        assert_eq!(
            fs::read(root.path().join("original.docx")).unwrap(),
            original
        );
        assert!(app.imported_name.is_none());
    }
    #[test]
    fn local_upload_validates_type_name_and_utf8() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(Some(root.path().into()));
        for name in [
            "",
            "../escape.txt",
            "nested/file.docx",
            "nested\\file.docx",
            "legacy.doc",
        ] {
            assert!(app
                .dispatch(
                    &json!({"command":"openFile","name":name,"data":STANDARD.encode(b"text")})
                )
                .is_err());
        }
        assert!(app
            .dispatch(
                &json!({"command":"openFile","name":"bad.txt","data":STANDARD.encode([0xff])})
            )
            .is_err());
        let opened = app.dispatch(&json!({"command":"openFile","name":"notes.TXT","data":STANDARD.encode("First line\n第二行")})).unwrap();
        assert_eq!(opened["document"]["content"].as_array().unwrap().len(), 2);
        assert!(app.path.is_none());
        assert!(app.protect_original);
        app.dispatch(&json!({"command":"new"})).unwrap();
        assert!(app.imported_name.is_none());
    }
    #[test]
    fn atomic_save_and_recover() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(Some(dir.path().into()));
        let doc = json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"保存测试"}]}]});
        app.dispatch(&json!({"command":"autosave","document":doc}))
            .unwrap();
        assert_eq!(
            app.dispatch(&json!({"command":"recover"})).unwrap()["document"],
            doc
        );
        let r = app
            .dispatch(&json!({"command":"savePath","path":"test.docx","document":doc}))
            .unwrap();
        assert!(r["bytes"].as_u64().unwrap() > 500);
        assert!(!app.recovery.exists());
        assert_eq!(
            app.dispatch(&json!({"command":"openPath","path":"test.docx"}))
                .unwrap()["document"],
            doc
        );
        assert!(app.test_path("/tmp/escape.docx").is_err());
    }
}
