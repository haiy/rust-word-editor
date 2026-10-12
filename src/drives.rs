//! Existing rclone-authorized drives. Secrets remain owned by rclone.
//! User requests cannot select an executable, shell command or arbitrary local path.
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const MAX_FILE: usize = 96 * 1024 * 1024;
const MAX_META: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Connection {
    id: String,
    provider: String,
    remote: String,
    folder: String,
    readonly: bool,
}
#[derive(Clone)]
pub struct Service {
    root: PathBuf,
    lock: Arc<Mutex<()>>,
}
struct Output {
    success: bool,
    code: Option<i32>,
    bytes: Vec<u8>,
    error: Vec<u8>,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn clean_path(raw: &str) -> Result<String> {
    if raw.len() > 1500
        || raw
            .chars()
            .any(|c| c.is_control() || ['\\', ':', '%'].contains(&c))
    {
        bail!("路径包含不支持的字符");
    }
    if raw.is_empty() {
        return Ok(String::new());
    }
    if raw.starts_with('/') || raw.ends_with('/') || raw.split('/').count() > 30 {
        bail!("请选择连接目录内的相对路径");
    }
    for part in raw.split('/') {
        if part.is_empty() || [".", ".."].contains(&part) || part.trim() != part || part.len() > 255
        {
            bail!("路径不能包含空段、. 或 ..");
        }
    }
    Ok(raw.into())
}
fn clean_name(raw: &str) -> Result<String> {
    let name = clean_path(raw)?;
    if name.is_empty() || name.contains('/') {
        bail!("请输入单个文件名");
    }
    Ok(name)
}
fn split_remote(raw: &str) -> Result<(&str, String)> {
    let (name, path) = raw
        .split_once(':')
        .context("请选择已配置的 rclone 远端（名称:路径）")?;
    if name.is_empty()
        || name.len() > 100
        || name.starts_with('-')
        || name
            .chars()
            .any(|c| c.is_control() || ['/', '\\', ':'].contains(&c))
    {
        bail!("远端名称无效");
    }
    Ok((name, clean_path(path)?))
}
fn join(root: &str, path: &str) -> String {
    if root.is_empty() {
        path.into()
    } else if path.is_empty() {
        root.into()
    } else {
        format!("{root}/{path}")
    }
}
fn destination(connection: &Connection, path: &str) -> Result<String> {
    let (name, base) = split_remote(&connection.remote)?;
    let folder = clean_path(&connection.folder)?;
    let path = clean_path(path)?;
    Ok(format!("{name}:{}", join(&join(&base, &folder), &path)))
}
fn executable() -> Option<PathBuf> {
    let mut candidates = vec![];
    if let Ok(current) = std::env::current_exe() {
        if let Some(parent) = current.parent() {
            candidates.push(parent.join("rclone"));
            if parent.file_name().is_some_and(|n| n == "deps") {
                if let Some(debug) = parent.parent() {
                    candidates.push(debug.join("rclone"));
                }
            }
        }
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/rclone"),
        PathBuf::from("/usr/local/bin/rclone"),
    ]);
    candidates.into_iter().find(|p| p.is_file())
}
fn run(arguments: &[String], limit: usize, seconds: u64) -> Result<Output> {
    let binary = executable().context("未找到 rclone；可先通过盘神或 rclone 配置网盘授权")?;
    let mut command = Command::new(binary);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Honor rclone's standard HTTP(S)_PROXY / NO_PROXY settings. Removing them
    // prevents Internet providers from connecting on proxy-dependent networks.
    let mut child = command.spawn().context("无法启动网盘连接工具")?;
    let stdout = child.stdout.take().context("读取网盘输出失败")?;
    let stderr = child.stderr.take().context("读取网盘状态失败")?;
    let out = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut b = vec![];
        stdout.take((limit + 1) as u64).read_to_end(&mut b)?;
        Ok(b)
    });
    let err = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut b = vec![];
        stderr.take(65536).read_to_end(&mut b)?;
        Ok(b)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(seconds) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            bail!("网盘连接超时，请检查当前网络或稍后重试");
        }
        thread::sleep(Duration::from_millis(20));
    };
    let bytes = out.join().map_err(|_| anyhow!("网盘输出中断"))??;
    let error = err.join().map_err(|_| anyhow!("网盘状态中断"))??;
    if bytes.len() > limit {
        bail!("网盘返回内容超过允许大小");
    }
    Ok(Output {
        success: status.success(),
        code: status.code(),
        bytes,
        error,
    })
}
fn error_kind(output: &Output) -> &'static str {
    // rclone lsjson --stat can report a missing object with no stderr.
    // Official exit codes 3 and 4 mean missing directory/file respectively.
    if matches!(output.code, Some(3 | 4)) {
        return "missing";
    }
    let error = String::from_utf8_lossy(&output.error).to_ascii_lowercase();
    if error.contains("401") || error.contains("unauthorized") || error.contains("authentication") {
        "auth"
    } else if error.contains("405")
        || error.contains("501")
        || error.contains("method not allowed")
        || error.contains("not supported")
        || error.contains("not implemented")
    {
        "unsupported"
    } else if error.contains("403")
        || error.contains("permission denied")
        || error.contains("forbidden")
    {
        "denied"
    } else if error.contains("404")
        || error.contains("not found")
        || error.contains("doesn't exist")
        || error.contains("directory not found")
    {
        "missing"
    } else if error.contains("timeout")
        || error.contains("connection refused")
        || error.contains("no route")
        || error.contains("no such host")
    {
        "network"
    } else {
        "service"
    }
}
fn service_error(output: &Output) -> anyhow::Error {
    anyhow!(match error_kind(output) {
        "auth" => "网盘授权已失效或认证被拒绝，请在盘神、OpenList 或 rclone 更新授权",
        "denied" => "当前网盘或目录拒绝此操作，可能只支持读取",
        "unsupported" => "当前网盘驱动不支持此操作，可继续浏览、下载与导入",
        "missing" => "网盘目录或文件不存在，请刷新列表",
        "network" => "网盘当前无法连接，请检查网络和网关状态",
        _ => "网盘服务未完成操作，请稍后重试或检查接入服务状态",
    })
}
fn configured() -> Result<Vec<Value>> {
    let output = run(&["listremotes".into(), "--json".into()], MAX_META, 8)?;
    if !output.success {
        return Err(service_error(&output));
    }
    let rows: Vec<Value> =
        serde_json::from_slice(&output.bytes).context("rclone 远端列表格式无效")?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name = row["name"].as_str()?;
            let kind = row["type"].as_str()?;
            Some(json!({"name":name,"type":kind}))
        })
        .collect())
}
fn validate_provider(provider: &str, backend: &str) -> Result<()> {
    let expected = match provider {
        "google-drive" => "drive",
        "dropbox" => "dropbox",
        "baidu" | "quark" | "nas" | "rclone" => return Ok(()),
        _ => bail!("不支持的网盘来源"),
    };
    if backend != expected {
        bail!("所选连接与网盘类型不匹配，请刷新授权列表并重新选择");
    }
    Ok(())
}
fn setup_command() -> Option<String> {
    executable().map(|path| {
        // Display-only POSIX shell quoting; this command is never executed by IPC.
        format!("'{}' config", path.to_string_lossy().replace('\'', "'\\''"))
    })
}
fn route_args(connection: &Connection) -> (Vec<String>, &'static str) {
    let flags = if connection.provider == "google-drive" {
        // Use editable Office exports, and preserve uploaded DOCX bytes instead
        // of a potentially lossy conversion into a native Google document.
        vec![
            "--drive-export-formats".into(),
            "docx,xlsx,pptx,svg".into(),
            "--drive-import-formats".into(),
            String::new(),
        ]
    } else {
        vec![]
    };
    (flags, "configured")
}
fn arguments(verb: &str, extra: &[&str], paths: &[String], route: &[String]) -> Vec<String> {
    let mut args = vec![
        verb.into(),
        "--contimeout".into(),
        "3s".into(),
        "--timeout".into(),
        "20s".into(),
        "--retries".into(),
        "1".into(),
        "--low-level-retries".into(),
        "1".into(),
        "--stats".into(),
        "0".into(),
        "--log-level".into(),
        "ERROR".into(),
    ];
    args.extend(route.iter().cloned());
    args.extend(extra.iter().map(|v| (*v).to_string()));
    args.push("--".into());
    args.extend(paths.iter().cloned());
    args
}
fn list_remote(path: &str, route: &[String]) -> Result<Vec<Value>> {
    let output = run(
        &arguments("lsjson", &["--max-depth", "1"], &[path.into()], route),
        MAX_META,
        35,
    )?;
    if !output.success {
        return Err(service_error(&output));
    }
    serde_json::from_slice(&output.bytes).context("网盘目录列表格式无效")
}
fn exists_remote(path: &str, route: &[String]) -> Result<bool> {
    let output = run(
        &arguments("lsjson", &["--stat"], &[path.into()], route),
        MAX_META,
        35,
    )?;
    if output.success {
        return Ok(true);
    }
    if error_kind(&output) == "missing" {
        return Ok(false);
    }
    Err(service_error(&output))
}
fn decode(raw: &str) -> Result<Vec<u8>> {
    let encoded = if raw.starts_with("data:") {
        let (h, d) = raw.split_once(',').context("文件 data URL 无效")?;
        if !h.ends_with(";base64") {
            bail!("文件必须使用 base64");
        }
        d
    } else {
        raw
    };
    if encoded.len() > MAX_FILE.div_ceil(3) * 4 {
        bail!("文件不能超过 96 MB");
    }
    let bytes = STANDARD.decode(encoded).context("文件 base64 数据损坏")?;
    if bytes.len() > MAX_FILE {
        bail!("文件不能超过 96 MB");
    }
    Ok(bytes)
}

impl Service {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            lock: Arc::new(Mutex::new(())),
        }
    }
    fn load(&self) -> Result<Vec<Connection>> {
        match fs::read(self.root.join("drives.json")) {
            Ok(bytes) => {
                if bytes.len() > 1024 * 1024 {
                    bail!("网盘连接配置过大");
                }
                let rows: Vec<Connection> =
                    serde_json::from_slice(&bytes).context("网盘连接配置损坏")?;
                if rows.len() > 50 {
                    bail!("网盘连接数量超出限制");
                }
                for row in &rows {
                    split_remote(&row.remote)?;
                    clean_path(&row.folder)?;
                }
                Ok(rows)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(error) => Err(error.into()),
        }
    }
    fn persist(&self, rows: &[Connection]) -> Result<()> {
        if rows.len() > 50 {
            bail!("最多保存 50 个网盘连接");
        }
        fs::create_dir_all(&self.root)?;
        let mut tmp = tempfile::NamedTempFile::new_in(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tmp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        tmp.write_all(&serde_json::to_vec_pretty(rows)?)?;
        tmp.as_file().sync_all()?;
        tmp.persist(self.root.join("drives.json"))
            .map_err(|error| error.error)?;
        Ok(())
    }
    fn store(&self, connection: Connection) -> Result<()> {
        let _guard = self.lock.lock().map_err(|_| anyhow!("网盘状态锁错误"))?;
        let mut rows = self.load()?;
        rows.retain(|r| r.id != connection.id);
        rows.push(connection);
        self.persist(&rows)
    }
    fn connection(&self, request: &Value) -> Result<Connection> {
        let id = text(request, "connectionId");
        self.load()?
            .into_iter()
            .find(|r| r.id == id)
            .context("请选择一个已连接的网盘")
    }
    fn discover(&self) -> Result<Value> {
        if executable().is_none() {
            return Ok(
                json!({"available":false,"remotes":[],"presets":[],"connections":self.load()?,"status":"尚未安装 rclone，Word 编辑与文件导入仍可使用"}),
            );
        }
        let remotes = configured()?;
        let presets: Vec<Value> = vec![];
        let version = run(&["version".into()], MAX_META, 5)?.bytes;
        let version = String::from_utf8_lossy(&version)
            .lines()
            .next()
            .unwrap_or("rclone")
            .to_string();
        Ok(
            json!({"available":true,"version":version,"setupCommand":setup_command(),"remotes":remotes,"presets":presets,"connections":self.load()?,
            "status":"复用已配置的网盘授权；连接会实际测试当前目录"}),
        )
    }
    fn connect(&self, request: &Value) -> Result<Value> {
        let provider = if text(request, "provider").is_empty() {
            "rclone"
        } else {
            text(request, "provider")
        };
        if !["baidu", "quark", "nas", "rclone", "google-drive", "dropbox"].contains(&provider) {
            bail!("不支持的网盘来源");
        }
        let raw_remote = text(request, "remote");
        if raw_remote.is_empty() {
            bail!("请选择已配置的 rclone 远端");
        }
        let (name, _) = split_remote(raw_remote)?;
        let remotes = configured()?;
        let remote = remotes
            .iter()
            .find(|r| text(r, "name").trim_end_matches(':') == name)
            .context("此远端尚未授权配置，请先在盘神、OpenList 或 rclone 完成连接")?;
        validate_provider(provider, text(remote, "type"))?;
        let folder = if let Some(folder) = request["folder"].as_str() {
            clean_path(folder)?
        } else if provider == "quark" {
            String::new()
        } else {
            "RustWord".into()
        };
        let readonly = request["readonly"].as_bool().unwrap_or(provider == "quark");
        let mut connection = Connection {
            id: String::new(),
            provider: provider.into(),
            remote: raw_remote.into(),
            folder,
            readonly,
        };
        let (route, route_name) = route_args(&connection);
        let mut write_failure = None;
        if request["createFolder"].as_bool().unwrap_or(!readonly)
            && !readonly
            && !connection.folder.is_empty()
        {
            let path = destination(&connection, "")?;
            let output = run(&arguments("mkdir", &[], &[path], &route), MAX_META, 35)?;
            if !output.success {
                let kind = error_kind(&output);
                if ["denied", "unsupported"].contains(&kind) {
                    connection.readonly = true;
                    connection.folder.clear();
                    write_failure = Some(kind);
                } else {
                    return Err(service_error(&output));
                }
            }
        }
        let mut entries = list_remote(&destination(&connection, "")?, &route);
        if entries.is_err() && connection.readonly && !connection.folder.is_empty() {
            connection.folder.clear();
            entries = list_remote(&destination(&connection, "")?, &route);
        }
        let entries = entries?;
        let id = digest(
            format!(
                "{}\0{}\0{}",
                connection.provider, connection.remote, connection.folder
            )
            .as_bytes(),
        );
        connection.id = format!("drive-{}", &id[..16]);
        self.store(connection.clone())?;
        Ok(
            json!({"connection":connection,"connected":true,"readable":true,"writable":write_failure.unwrap_or(if connection.readonly{"readonly"}else{"unknown"}),
            "route":route_name,"entryCount":entries.len(),"status":match write_failure {Some("denied")=>"目录写入被服务拒绝，已连接为只读；可浏览与下载",Some("unsupported")=>"网盘驱动不支持目录写入，已连接为只读；可浏览、下载与导入",_ if connection.readonly=>"已实际读取目录，启用只读保护",_=>"已实际读取目录；上传后会回读校验文件内容"}}),
        )
    }
    fn list(&self, request: &Value) -> Result<Value> {
        let connection = self.connection(request)?;
        let path = clean_path(text(request, "path"))?;
        let (route, route_name) = route_args(&connection);
        let rows = list_remote(&destination(&connection, &path)?, &route)?;
        let total = rows.len();
        let mut directories = vec![];
        let mut files = vec![];
        for row in rows.into_iter().take(1000) {
            let name = row["Name"].as_str().unwrap_or("");
            if clean_name(name).is_err() {
                continue;
            }
            let item = json!({"name":name,"path":join(&path,name),"bytes":row["Size"].as_i64().unwrap_or(0).max(0),
                "modified":row["ModTime"],"mime":row["MimeType"],"readonly":connection.readonly});
            if row["IsDir"] == true {
                directories.push(item)
            } else {
                files.push(item)
            }
        }
        directories.sort_by_key(|v| text(v, "name").to_lowercase());
        files.sort_by_key(|v| text(v, "name").to_lowercase());
        Ok(
            json!({"connection":connection,"path":path,"directories":directories,"files":files,"readonly":connection.readonly,
            "writable":if connection.readonly{"readonly"}else{"unknown"},"route":route_name,"truncated":total>1000,
            "status":if connection.readonly{"目录读取成功；当前连接为只读"}else{"目录读取成功，上传会保留已有文件并回读校验"}}),
        )
    }
    fn download_bytes(
        &self,
        connection: &Connection,
        path: &str,
        route: &[String],
    ) -> Result<Vec<u8>> {
        let output = run(
            &arguments("cat", &[], &[destination(connection, path)?], route),
            MAX_FILE,
            120,
        )?;
        if !output.success {
            return Err(service_error(&output));
        }
        Ok(output.bytes)
    }
    fn upload(&self, request: &Value) -> Result<Value> {
        let mut connection = self.connection(request)?;
        if connection.readonly {
            bail!("此连接启用了只读保护，不能上传；浏览、下载和导入仍可使用");
        }
        let folder = clean_path(text(request, "path"))?;
        let name = clean_name(text(request, "name"))?;
        let path = join(&folder, &name);
        let bytes = decode(text(request, "data"))?;
        let (route, route_name) = route_args(&connection);
        let target = destination(&connection, &path)?;
        if exists_remote(&target, &route)? {
            bail!("网盘已有同名文件，请换个名称以保留已有内容");
        }
        fs::create_dir_all(&self.root)?;
        let mut source = tempfile::NamedTempFile::new_in(&self.root)?;
        source.write_all(&bytes)?;
        source.as_file().sync_all()?;
        let output = run(
            &arguments(
                "copyto",
                &["--ignore-existing", "--immutable", "--no-traverse"],
                &[source.path().to_string_lossy().into(), target],
                &route,
            ),
            MAX_META,
            120,
        )?;
        if !output.success {
            if ["denied", "unsupported"].contains(&error_kind(&output)) {
                connection.readonly = true;
                self.store(connection)?;
            }
            return Err(service_error(&output));
        }
        let local_hash = digest(&bytes);
        let readback = self.download_bytes(&connection, &path, &route);
        match readback {
            Ok(downloaded) => {
                if digest(&downloaded) != local_hash {
                    bail!("上传后读取的内容校验不一致，请保留本地文件并检查网盘");
                }
                Ok(
                    json!({"connectionId":connection.id,"name":name,"path":path,"bytes":bytes.len(),"sha256":local_hash,
                    "uploaded":true,"readbackVerified":true,"route":route_name,"status":"已上传并从网盘回读，SHA-256 完全一致"}),
                )
            }
            Err(_) => Ok(
                json!({"connectionId":connection.id,"name":name,"path":path,"bytes":bytes.len(),"sha256":local_hash,
                "uploaded":true,"readbackVerified":false,"route":route_name,"status":"服务已接受上传，当前暂未能回读；请保留本地文件并稍后刷新确认"}),
            ),
        }
    }
    pub fn dispatch(&self, action: &str, request: &Value) -> Result<Value> {
        match action {
            "driveDiscover" => self.discover(),
            "driveConnect" => self.connect(request),
            "driveList" => self.list(request),
            "driveUpload" => self.upload(request),
            "driveDownload" => {
                let connection = self.connection(request)?;
                let path = if text(request, "path").is_empty() {
                    clean_name(text(request, "name"))?
                } else {
                    clean_path(text(request, "path"))?
                };
                if path.is_empty() {
                    bail!("请选择需要下载的文件");
                }
                let (route, route_name) = route_args(&connection);
                let bytes = self.download_bytes(&connection, &path, &route)?;
                Ok(
                    json!({"connectionId":connection.id,"name":path.rsplit('/').next().unwrap_or("文件"),"path":path,
                    "data":STANDARD.encode(&bytes),"bytes":bytes.len(),"sha256":digest(&bytes),"route":route_name,"status":"已从网盘读取文件"}),
                )
            }
            "driveDisconnect" => {
                let _guard = self.lock.lock().map_err(|_| anyhow!("网盘状态锁错误"))?;
                let id = text(request, "connectionId");
                let mut rows = self.load()?;
                let before = rows.len();
                rows.retain(|r| r.id != id);
                if before == rows.len() {
                    bail!("未找到此网盘连接");
                }
                self.persist(&rows)?;
                Ok(json!({"ok":true,"status":"已移除本应用连接，网盘文件和 rclone 授权仍保留"}))
            }
            _ => bail!("未知网盘操作"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_providers_require_the_correct_backend() {
        assert!(validate_provider("google-drive", "drive").is_ok());
        assert!(validate_provider("dropbox", "dropbox").is_ok());
        for provider in ["google-drive", "dropbox"] {
            for backend in ["webdav", "alias", "local", ""] {
                assert!(validate_provider(provider, backend).is_err());
            }
        }
        assert!(validate_provider("google-drive", "dropbox").is_err());
        assert!(validate_provider("dropbox", "drive").is_err());
        assert!(validate_provider("rclone", "alias").is_ok());
        assert!(validate_provider("unknown", "drive").is_err());
    }
    #[test]
    fn remote_paths_cannot_escape_or_inject_options() {
        for bad in [
            "../secret",
            "a/../secret",
            "/absolute",
            "a//b",
            "a\\b",
            "%2e%2e/x",
            "x:y",
            "a\narg",
        ] {
            assert!(clean_path(bad).is_err(), "{bad}");
        }
        assert!(split_remote("--config:/tmp/private").is_err());
        let connection = Connection {
            id: "test".into(),
            provider: "rclone".into(),
            remote: "sample:base".into(),
            folder: "RustWord".into(),
            readonly: false,
        };
        assert_eq!(
            destination(&connection, "中文/说明.docx").unwrap(),
            "sample:base/RustWord/中文/说明.docx"
        );
        let args = arguments(
            "cat",
            &[],
            &["sample:base/RustWord/name`echo`.docx".into()],
            &[],
        );
        assert_eq!(args.last().unwrap(), "sample:base/RustWord/name`echo`.docx");
        assert_eq!(args[args.len() - 2], "--");
    }
    #[test]
    fn saved_connections_contain_no_credentials_and_disconnect_only_local() {
        let temp = tempfile::tempdir().unwrap();
        let service = Service::new(temp.path());
        let connection = Connection {
            id: "drive-test".into(),
            provider: "quark".into(),
            remote: "configured:quark".into(),
            folder: String::new(),
            readonly: true,
        };
        service.store(connection).unwrap();
        let value: Value =
            serde_json::from_slice(&fs::read(temp.path().join("drives.json")).unwrap()).unwrap();
        let keys = value[0]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(keys.len(), 5);
        assert!(keys
            .iter()
            .all(|k| ["id", "provider", "remote", "folder", "readonly"].contains(&k.as_str())));
        assert!(service
            .dispatch(
                "driveUpload",
                &json!({"connectionId":"drive-test","name":"x.docx","data":"eA=="})
            )
            .unwrap_err()
            .to_string()
            .contains("只读"));
        service
            .dispatch("driveDisconnect", &json!({"connectionId":"drive-test"}))
            .unwrap();
        assert!(service.load().unwrap().is_empty());
    }
    #[test]
    fn missing_remote_stat_uses_exit_code_even_when_stderr_is_empty() {
        for code in [3, 4] {
            let output = Output {
                success: false,
                code: Some(code),
                bytes: b"null".to_vec(),
                error: vec![],
            };
            assert_eq!(error_kind(&output), "missing");
        }
        let denied = Output {
            success: false,
            code: Some(1),
            bytes: vec![],
            error: b"HTTP 403".to_vec(),
        };
        assert_eq!(error_kind(&denied), "denied");
    }
    #[test]
    fn service_errors_never_echo_remote_response_secrets() {
        let response = Output {
            success: false,
            code: Some(1),
            bytes: vec![],
            error: b"HTTP 403 Authorization: Bearer TEST_SECRET remote/private_name".to_vec(),
        };
        assert_eq!(error_kind(&response), "denied");
        let error = service_error(&response).to_string();
        assert!(!error.contains("TEST_SECRET") && !error.contains("private_name"));
        let unsupported = Output {
            success: false,
            code: Some(1),
            bytes: vec![],
            error: b"HTTP 405 Method Not Allowed private_name cookie=TEST_SECRET".to_vec(),
        };
        assert_eq!(error_kind(&unsupported), "unsupported");
        let error = service_error(&unsupported).to_string();
        assert!(error.contains("不支持"));
        assert!(!error.contains("TEST_SECRET") && !error.contains("private_name"));
        assert!(decode("data:text/plain,hello").is_err());
        assert!(decode("bad base64").is_err());
    }
}
