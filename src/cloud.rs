//! Personal storage adapters. A directory can be an iCloud/Nextcloud sync folder;
//! WebDAV talks to the configured remote service and never invents a quota.
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_FILE: usize = 96 * 1024 * 1024;
#[derive(Default, Serialize, Deserialize)]
struct Settings {
    provider: String,
    folder: String,
    endpoint: String,
    username: String,
    password: String,
}
#[derive(Clone)]
pub struct Storage {
    root: PathBuf,
}
impl Storage {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
        }
    }
    fn load(&self) -> Result<Settings> {
        match fs::read(self.root.join("cloud.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).context("云空间配置损坏")?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn settings(&self) -> Result<Value> {
        let s = self.load()?;
        Ok(
            json!({"provider":s.provider,"folder":s.folder,"endpoint":s.endpoint,"username":s.username,"hasPassword":!s.password.is_empty(),"capacity":"由接入的服务或磁盘提供","configured":!s.provider.is_empty()}),
        )
    }
    pub fn set_folder(&self, folder: &Path, provider: &str) -> Result<Value> {
        if !folder.is_dir() {
            bail!("请选择已经存在的同步文件夹");
        }
        let folder = folder.canonicalize()?;
        let s = Settings {
            provider: provider.into(),
            folder: folder.to_string_lossy().into(),
            ..Default::default()
        };
        self.persist(&s)?;
        self.settings()
    }
    pub fn set_icloud(&self) -> Result<Value> {
        let home = std::env::var_os("HOME").context("无法找到用户目录")?;
        let drive = PathBuf::from(home).join("Library/Mobile Documents/com~apple~CloudDocs");
        if !drive.is_dir() {
            bail!("本机未启用 iCloud Drive。可选择已有同步文件夹，或配置 WebDAV");
        }
        let folder = drive.join("Rust Word");
        fs::create_dir_all(&folder)?;
        self.set_folder(&folder, "icloud")
    }
    fn persist(&self, s: &Settings) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        let mut tmp = tempfile::NamedTempFile::new_in(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tmp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        tmp.write_all(&serde_json::to_vec(s)?)?;
        tmp.as_file().sync_all()?;
        tmp.persist(self.root.join("cloud.json"))
            .map_err(|e| e.error)?;
        Ok(())
    }
    pub fn save_settings(&self, value: &Value) -> Result<Value> {
        let old = self.load()?;
        let endpoint = value["endpoint"].as_str().unwrap_or("").trim().to_string();
        endpoint_url(&endpoint)?;
        let username = value["username"].as_str().unwrap_or("").to_string();
        if username.len() > 500 {
            bail!("用户名过长");
        }
        let password = if value["clearPassword"] == true {
            String::new()
        } else if let Some(p) = value["password"].as_str().filter(|p| !p.is_empty()) {
            p.into()
        } else if old.endpoint == endpoint && old.username == username {
            old.password
        } else {
            String::new()
        };
        if password.len() > 4096 {
            bail!("密码过长");
        }
        self.persist(&Settings {
            provider: "webdav".into(),
            endpoint,
            username,
            password,
            ..Default::default()
        })?;
        self.settings()
    }
    fn client(s: &Settings) -> Result<Client> {
        endpoint_url(&s.endpoint)?;
        Ok(Client::builder()
            .timeout(Duration::from_secs(45))
            .connect_timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build()?)
    }
    fn request(
        s: &Settings,
        method: Method,
        url: Url,
    ) -> Result<reqwest::blocking::RequestBuilder> {
        let req = Self::client(s)?.request(method, url);
        Ok(if s.username.is_empty() && s.password.is_empty() {
            req
        } else {
            req.basic_auth(&s.username, Some(&s.password))
        })
    }
    pub fn dispatch(&self, action: &str, req: &Value) -> Result<Value> {
        if action == "cloudSettings" {
            return self.settings();
        }
        if action == "saveCloudSettings" {
            return self.save_settings(&req["settings"]);
        }
        let s = self.load()?;
        if s.provider.is_empty() {
            bail!("请先连接 iCloud、同步文件夹或 WebDAV 云空间")
        }
        match action {
            "cloudList" | "cloudTest" => {
                let files = if s.provider == "webdav" {
                    self.list_webdav(&s)?
                } else {
                    self.list_folder(&s)?
                };
                Ok(
                    json!({"files":files,"provider":s.provider,"status":if s.provider=="webdav" {"已连接远程 WebDAV"} else if s.provider=="icloud" {"已写入 iCloud Drive 目录，同步状态由系统管理"} else {"同步文件夹已连接，远程同步由对应服务管理"},"capacity":"由接入服务提供"}),
                )
            }
            "cloudUpload" => {
                let name = req["name"].as_str().context("缺少文件名")?;
                validate_name(name)?;
                let bytes = decode(req["data"].as_str().context("缺少文件内容")?)?;
                if s.provider == "webdav" {
                    let result = Self::request(&s, Method::PUT, child_url(&s.endpoint, name)?)?
                        .header("If-None-Match", "*")
                        .body(bytes.clone())
                        .send()?;
                    if result.status().as_u16() == 412 {
                        bail!("云端已有同名文件。请换个名称，避免覆盖")
                    }
                    if !result.status().is_success() {
                        bail!("云端上传失败（HTTP {}）", result.status().as_u16())
                    }
                } else {
                    let folder = folder(&s)?;
                    let dest = folder.join(name);
                    if dest.exists() {
                        bail!("同步文件夹已有同名文件，请换个名称")
                    }
                    let mut tmp = tempfile::NamedTempFile::new_in(&folder)?;
                    tmp.write_all(&bytes)?;
                    tmp.as_file().sync_all()?;
                    tmp.persist_noclobber(&dest).map_err(|e| e.error)?;
                }
                Ok(json!({"name":name,"bytes":bytes.len(),"provider":s.provider}))
            }
            "cloudDownload" => {
                let name = req["name"].as_str().context("缺少文件名")?;
                validate_name(name)?;
                let bytes = if s.provider == "webdav" {
                    let mut result =
                        Self::request(&s, Method::GET, child_url(&s.endpoint, name)?)?.send()?;
                    if !result.status().is_success() {
                        bail!("云端下载失败（HTTP {}）", result.status().as_u16())
                    }
                    if result.content_length().unwrap_or(0) > MAX_FILE as u64 {
                        bail!("文件超过 96 MB")
                    }
                    let mut bytes = Vec::new();
                    result
                        .by_ref()
                        .take((MAX_FILE + 1) as u64)
                        .read_to_end(&mut bytes)?;
                    bytes
                } else {
                    let base = folder(&s)?;
                    let path = base.join(name).canonicalize()?;
                    if path.parent() != Some(base.as_path()) || !path.is_file() {
                        bail!("文件位置不在已连接的同步文件夹内")
                    }
                    if fs::metadata(&path)?.len() > MAX_FILE as u64 {
                        bail!("文件超过 96 MB")
                    }
                    fs::read(path)?
                };
                if bytes.len() > MAX_FILE {
                    bail!("文件超过 96 MB")
                }
                Ok(json!({"name":name,"data":STANDARD.encode(bytes)}))
            }
            _ => bail!("未知云空间操作"),
        }
    }
    fn list_folder(&self, s: &Settings) -> Result<Vec<Value>> {
        let base = folder(s)?;
        let mut files = vec![];
        for entry in fs::read_dir(&base)?.take(5000) {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if validate_name(&name).is_err() {
                continue;
            }
            let meta = entry.metadata()?;
            if meta.is_file() && !entry.file_type()?.is_symlink() {
                files.push(json!({"name":name,"bytes":meta.len(),"modified":meta.modified().ok().and_then(|v|v.duration_since(UNIX_EPOCH).ok()).map(|v|v.as_secs().to_string()).unwrap_or_default()}))
            }
        }
        files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        Ok(files)
    }
    fn list_webdav(&self, s: &Settings) -> Result<Vec<Value>> {
        let result=Self::request(s,Method::from_bytes(b"PROPFIND")?,endpoint_url(&s.endpoint)?)?.header("Depth","1").header("Content-Type","application/xml; charset=utf-8").body(r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:displayname/><d:getcontentlength/><d:getlastmodified/><d:resourcetype/></d:prop></d:propfind>"#).send()?;
        if !result.status().is_success() {
            bail!(
                "云空间无法读取文件列表（HTTP {}）",
                result.status().as_u16()
            )
        }
        let mut text = String::new();
        result.take(4 * 1024 * 1024 + 1).read_to_string(&mut text)?;
        if text.len() > 4 * 1024 * 1024 {
            bail!("远端文件列表过大，请连接一个专用文件夹")
        }
        let xml = roxmltree::Document::parse(&text).context("WebDAV 返回的文件列表格式无效")?;
        let mut files = vec![];
        let base = endpoint_url(&s.endpoint)?;
        for node in xml
            .descendants()
            .filter(|n| n.tag_name().name() == "response")
        {
            if node
                .descendants()
                .any(|n| n.tag_name().name() == "collection")
            {
                continue;
            }
            let text_of = |tag: &str| {
                node.descendants()
                    .find(|n| n.tag_name().name() == tag)
                    .and_then(|n| n.text())
                    .unwrap_or("")
            };
            let href = text_of("href");
            let Ok(url) = base.join(href) else { continue };
            if url.origin() != base.origin() {
                continue;
            }
            let prefix = base.path();
            let Some(segment) = url.path().strip_prefix(prefix) else {
                continue;
            };
            if segment.contains('/') || segment.is_empty() {
                continue;
            }
            let name = percent_decode(segment)?;
            if validate_name(&name).is_err() {
                continue;
            }
            let status = text_of("status");
            if !status.is_empty() && !status.contains(" 200 ") {
                continue;
            }
            files.push(json!({"name":name,"bytes":text_of("getcontentlength").parse::<u64>().unwrap_or(0),"modified":text_of("getlastmodified")}));
            if files.len() >= 2000 {
                break;
            }
        }
        files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        Ok(files)
    }
}
fn folder(s: &Settings) -> Result<PathBuf> {
    let path = PathBuf::from(&s.folder)
        .canonicalize()
        .context("同步文件夹已移动或不可用")?;
    if !path.is_dir() {
        bail!("同步位置不是文件夹")
    };
    Ok(path)
}
fn endpoint_url(endpoint: &str) -> Result<Url> {
    let mut url = Url::parse(endpoint).context("请输入完整 WebDAV HTTPS 地址")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("地址不能包含凭据、查询参数或片段，请单独填写用户名和密码")
    }
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(loopback && url.scheme() == "http") {
        bail!("远程云空间需要 HTTPS，localhost 测试可使用 HTTP")
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()))
    };
    Ok(url)
}
fn child_url(base: &str, name: &str) -> Result<Url> {
    validate_name(name)?;
    let mut url = endpoint_url(base)?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("云端地址不支持文件路径"))?
        .pop_if_empty()
        .push(name);
    Ok(url)
}
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.chars().count() > 160
        || name.starts_with('.')
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
    {
        bail!("文件名无效")
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ![
        "docx", "pptx", "potx", "pdf", "png", "jpg", "jpeg", "webp", "txt",
    ]
    .contains(&ext.as_str())
    {
        bail!("云空间支持 Word、PPT、PDF、图片及文本文件")
    };
    Ok(())
}
fn percent_decode(s: &str) -> Result<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let n = u8::from_str_radix(&s[i + 1..i + 3], 16)?;
            out.push(n);
            i += 3
        } else {
            out.push(bytes[i]);
            i += 1
        }
    }
    Ok(String::from_utf8(out)?)
}
pub fn decode(data: &str) -> Result<Vec<u8>> {
    let encoded = data
        .split_once(',')
        .filter(|_| data.starts_with("data:"))
        .map(|v| v.1)
        .unwrap_or(data);
    if encoded.len() > MAX_FILE * 4 / 3 + 16 {
        bail!("文件超过 96 MB")
    };
    let bytes = STANDARD
        .decode(encoded)
        .context("文件内容不是有效 base64")?;
    if bytes.len() > MAX_FILE {
        bail!("文件超过 96 MB")
    };
    Ok(bytes)
}
pub fn suggested_name(title: &str, ext: &str) -> String {
    let safe: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(90)
        .collect();
    let safe = safe.trim_matches('.');
    format!(
        "{}-{}.{ext}",
        if safe.is_empty() { "文档" } else { safe },
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_upload_read_persist_and_noclobber() {
        let root = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let storage = Storage::new(root.path());
        storage.set_folder(folder.path(), "folder").unwrap();
        let req = json!({"name":"测试.docx","data":STANDARD.encode(b"bytes")});
        storage.dispatch("cloudUpload", &req).unwrap();
        assert!(storage.dispatch("cloudUpload", &req).is_err());
        assert_eq!(
            storage.dispatch("cloudDownload", &req).unwrap()["data"],
            STANDARD.encode(b"bytes")
        );
        assert_eq!(
            Storage::new(root.path())
                .dispatch("cloudList", &json!({}))
                .unwrap()["files"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(storage
            .dispatch("cloudDownload", &json!({"name":"../secret.docx"}))
            .is_err())
    }
    #[test]
    fn endpoint_and_names() {
        assert!(endpoint_url("http://example.com/dav").is_err());
        assert!(endpoint_url("https://x:y@example.com/dav").is_err());
        assert!(endpoint_url("http://127.0.0.1:123/dav").is_ok());
        assert_eq!(
            child_url("https://example.com/dav/", "中文 文件.docx")
                .unwrap()
                .path(),
            "/dav/%E4%B8%AD%E6%96%87%20%E6%96%87%E4%BB%B6.docx"
        );
        assert!(validate_name("file.exe").is_err())
    }
}
