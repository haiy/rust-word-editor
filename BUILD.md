# Building Rust Word

[English](BUILD.md) · [简体中文](BUILD.zh-CN.md)

## Requirements

The supported release target is macOS 12+ on Apple Silicon. Install Rust (see `rust-toolchain.toml`), Node.js, Python 3.11+ and Xcode command-line tools. The public source tree includes all original starter decks and themes; no private resource download is required.

```bash
npm ci
npm run build
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
python3 scripts/build-media.py target/release/rust-word-media
```

Always rebuild the UI before Rust: the executable embeds `dist/index.html`. Both interface languages are bundled offline. Missing English catalog entries cause the UI build to fail. `node scripts/localization.mjs --extract` inventories source messages for translators; no model or network call runs during the build.

## Optional components

- **PPTX preview/image export:** install LibreOffice in `/Applications/LibreOffice.app`, or set `RUST_WORD_SOFFICE` to its `soffice` executable.
- **Configured cloud remotes:** install rclone and run `rclone config` yourself. The application reads remote names, never embeds the configuration in documents or application packages.
- **AI:** configure your own compatible HTTPS API or install and authenticate Codex CLI. GPU service settings are user-provided; the app does not create SSH tunnels or connect to a developer's server.

## Package a complete application

```bash
export RUST_WORD_LIBREOFFICE_APP=/Applications/LibreOffice.app
export RUST_WORD_RCLONE="$(command -v rclone)"
export RUST_WORD_RCLONE_LICENSE=/path/to/rclone/COPYING
python3 scripts/package-macos.py
python3 scripts/verify-app.py
```

The packager creates a staging app, includes component licenses, performs local ad-hoc signing and verification, then replaces its build output. It does not overwrite installed applications, user configuration or an open document. It creates no login/startup service. Apple notarization requires a separate developer signing workflow.

## Tests

`cargo test` covers document codecs, original templates, validation and credential redaction. Tests use synthetic files and disposable directories. The explicitly ignored live-model test requires your own authenticated AI service and should not be part of routine builds.

For protocol or browser tests:

```bash
mkdir -p test-output/sandbox
target/release/rust-word --test-server 127.0.0.1:18772 test-output/sandbox
```

Keep this process in the current terminal/session. The test server has no authentication and must remain on loopback. Some older UI scripts require an authorized codex-chrome test bridge configured on your machine; they contain no credentials. The bilingual release smoke test uses Playwright CLI. `node --test tests/localization.test.mjs` checks locale compilation, escaping and document-text preservation.

To regenerate the original PPTX assets, install `python-pptx` in a disposable virtual environment and run `python3 scripts/create-public-templates.py`. Template content and layout are original and MIT-licensed.
