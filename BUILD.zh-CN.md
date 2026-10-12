# Rust Word 构建

[English](BUILD.md) · [简体中文](BUILD.zh-CN.md)

支持 macOS 12+ Apple Silicon。需要 Rust、Node.js、Python 3.11+、Xcode 命令行工具。原创模板已随源码提供。

```bash
npm ci
npm run build
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
python3 scripts/build-media.py target/release/rust-word-media
```

Rust 嵌入 `dist/index.html`，因此每次修改 UI 后需先重新打包。中英文均离线内嵌；英文词条缺失会让构建失败。构建不调用大模型。

PPTX 预览与图片导出需要 LibreOffice，可安装到 `/Applications/LibreOffice.app`，或用 `RUST_WORD_SOFFICE` 指定 soffice。网盘需自行安装并配置 rclone；AI 需自己的兼容接口或已登录 Codex CLI。公开版不会连接开发者服务器或自动建立 SSH 隧道。

完整打包时设置 `RUST_WORD_LIBREOFFICE_APP`、`RUST_WORD_RCLONE` 与 `RUST_WORD_RCLONE_LICENSE`，然后运行 `python3 scripts/package-macos.py` 和 `python3 scripts/verify-app.py`。打包程序先生成临时应用、附许可并验证签名，再替换构建输出；不覆盖用户文档和配置，不创建自启动。Apple 公证需另行配置开发者签名。

测试服务只绑定 `127.0.0.1`，使用一次性目录。部分旧 UI 脚本需要自己配置的 codex-chrome 测试桥；新的双语发布烟测使用 Playwright。具体命令见 [英文构建说明](BUILD.md)。

重新生成原创 PPTX 需在隔离 Python 环境安装 `python-pptx`，执行 `python3 scripts/create-public-templates.py`。

网盘测试：`python3 tests/drive-providers.test.py`，需 release 构建和 rclone，不读取个人账号。首次授权见[云存储接入说明](CLOUD_STORAGE.zh-CN.md)。
