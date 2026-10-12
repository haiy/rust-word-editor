use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Library {
    root: PathBuf,
}
impl Library {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.join("materials"),
        }
    }
    pub fn dispatch(&self, action: &str, req: &Value) -> Result<Value> {
        fs::create_dir_all(&self.root)?;
        match action {
            "materialList" => {
                let mut entries = vec![];
                for entry in fs::read_dir(&self.root)?.take(4000) {
                    let entry = entry?;
                    if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
                        continue;
                    }
                    let meta: Value = serde_json::from_slice(&fs::read(entry.path())?)?;
                    entries.push(meta);
                }
                entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                Ok(json!({"materials":entries}))
            }
            "materialSave" => {
                let bytes = crate::cloud::decode(req["data"].as_str().context("缺少图片")?)?;
                if bytes.len() > 32 * 1024 * 1024 {
                    bail!("素材图片最多 32 MB")
                }
                let format = image::guess_format(&bytes).context("请选择有效图片")?;
                let ext = match format {
                    image::ImageFormat::Png => "png",
                    image::ImageFormat::Jpeg => "jpg",
                    image::ImageFormat::WebP => "webp",
                    image::ImageFormat::Gif => "gif",
                    _ => bail!("素材库支持 PNG、JPEG、WebP、GIF"),
                };
                let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes));
                reader.set_format(format);
                let (width, height) = reader.into_dimensions()?;
                if width as u64 * height as u64 > 80_000_000 {
                    bail!("素材图片像素过大")
                }
                let id = format!("{:x}", Sha256::digest(&bytes));
                let name = req["name"].as_str().unwrap_or("图片素材").trim();
                if name.is_empty() || name.chars().count() > 160 {
                    bail!("素材名称需为 1 至 160 字")
                }
                let category = req["category"].as_str().unwrap_or("我的素材");
                if category.chars().count() > 40 {
                    bail!("素材分类过长")
                }
                let meta = json!({"id":id,"name":name,"category":category,"ext":ext,"width":width,"height":height,"bytes":bytes.len()});
                if !self.root.join(format!("{id}.json")).exists()
                    && fs::read_dir(&self.root)?
                        .filter_map(Result::ok)
                        .filter(|e| e.path().extension().and_then(|v| v.to_str()) == Some("json"))
                        .count()
                        >= 2000
                {
                    bail!("素材库已达 2000 张，请清理后再添加")
                }
                atomic(&self.root.join(format!("{id}.{ext}")), &bytes)?;
                atomic(
                    &self.root.join(format!("{id}.json")),
                    &serde_json::to_vec(&meta)?,
                )?;
                Ok(meta)
            }
            "materialGet" | "materialDelete" => {
                let id = req["materialId"].as_str().context("缺少素材 ID")?;
                if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                    bail!("素材 ID 无效")
                }
                let meta_path = self.root.join(format!("{id}.json"));
                let meta: Value =
                    serde_json::from_slice(&fs::read(&meta_path).context("未找到素材")?)?;
                let ext = meta["ext"].as_str().context("素材元数据损坏")?;
                if !["png", "jpg", "gif", "webp"].contains(&ext) {
                    bail!("素材格式无效")
                }
                let path = self.root.join(format!("{id}.{ext}"));
                if action == "materialDelete" {
                    fs::remove_file(meta_path)?;
                    fs::remove_file(path)?;
                    return Ok(json!({"ok":true}));
                }
                if fs::metadata(&path)?.len() > 32 * 1024 * 1024 {
                    bail!("素材过大")
                }
                let mime = if ext == "jpg" { "jpeg" } else { ext };
                Ok(
                    json!({"data":format!("data:image/{mime};base64,{}",STANDARD.encode(fs::read(path)?)),"name":meta["name"],"width":meta["width"],"height":meta["height"]}),
                )
            }
            _ => bail!("未知素材库操作"),
        }
    }
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().context("保存位置无效")?)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
