# Rust Word

[English](README.md) · [简体中文](README.zh-CN.md)

以 Rust 为文档引擎的 macOS 办公编辑器，支持真实 DOCX 编辑保存、AI 写作、PPT、PDF、图片工具及网盘连接。

**[下载 macOS Apple Silicon 版](https://github.com/haiy/rust-word-editor/releases/latest)** · [构建说明](BUILD.zh-CN.md) · [使用指南](docs/USER_GUIDE.zh-CN.md)

## 功能

- 文字编辑：字体、段落、标题、列表、表格、图片、查找替换、页边距、草稿恢复及 DOCX 读写。
- 中英双语：顶部切换 English / 中文，包含原生菜单、工具对话框和原创写作模板；切换界面不会翻译或改写现有正文。
- AI 辅助：选区润色与纠错、全文翻译、排版建议和 PPT 大纲，预览后再采用。使用自己的兼容接口或本机已登录的 Codex CLI。
- 演示文稿：6 种原创内容结构、12 份中英文原创可编辑 PPTX、3 个主题；导入自己的 PPTX，编辑文字和页面，保留已有对象，导出 PPTX 或图片。
- PDF 与图片：PDF 页面整理、旋转、合并、拆分、批注、文字提取和 OCR；图片裁剪、旋转、缩放、调色与素材库。
- 存储：本机目录、iCloud Drive、WebDAV 和已配置的 rclone 远端，可接入自己的 NAS、百度和夸克。写入能力以驱动支持及账号权限为准。

## 开始使用

从 [Releases](https://github.com/haiy/rust-word-editor/releases/latest) 下载 `Rust-Word-0.4.0-macOS-arm64.zip`，对照 `SHA256SUMS.txt` 检查文件。解压后打开 Rust Word.app；更新前保存文档并退出旧版。

首启跟随系统语言，顶部可切换 English / 中文。切换保留当前 Word、PPT、PDF 与图片，但会重新加载界面，清空撤销历史及尚未应用的 AI 预览；确认框会先说明。

应用附 PDF/OCR 助手、rclone 和 LibreOffice 渲染组件，采用本机签名，尚未做 Apple 公证。遇到 Gatekeeper 提示时，请在系统“隐私与安全性”中检查应用，不要关闭系统整体保护。

AI 需要自己的账号或服务，普通编辑可离线使用。公开仓库及安装包不含 API Key、账号令牌、Cookie、个人网盘配置或私人研究模板。

## 构建

macOS 安装 Rust、Node.js、Python 3.11+ 和 Xcode 命令行工具后：

```bash
git clone https://github.com/haiy/rust-word-editor.git
cd rust-word-editor
npm ci
npm run build
cargo test
cargo build --release
python3 scripts/build-media.py target/release/rust-word-media
cargo run --release
```

原创模板资源随源码提供，不需要下载私人资源。完整打包步骤见 [BUILD.zh-CN.md](BUILD.zh-CN.md)。

## 能力边界

不支持旧 `.doc`、宏、完整修订和与 Microsoft Office 完全一致的分页。复杂 DOCX 可能需要另存副本来保护未支持的对象。PDF 转 Word 重建文字流，不保证复杂版式复原。PPTX 保留已有图形，但尚未开放全部形状属性编辑；导入 PPTX 的图片导出需 LibreOffice。部分夸克驱动仅支持读取。

源码及原创模板采用 [MIT](LICENSE)；第三方组件保留各自许可，详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。本项目与 Microsoft、WPS 无隶属关系。
