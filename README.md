# Rust Word

[English](README.md) · [简体中文](README.zh-CN.md)

An offline-first macOS word processor with a Rust document engine, editable DOCX files, AI-assisted writing, presentation tools, PDF utilities and cloud-drive connections.

**[Download for macOS Apple Silicon](https://github.com/haiy/rust-word-editor/releases/latest)** · [Build from source](BUILD.md) · [User guide](docs/USER_GUIDE.md) · [Security](SECURITY.md)

## Features

- **Write and edit:** rich text, headings, lists, tables, embedded images, search and replace, page settings, draft recovery, and real DOCX import/export.
- **English and Simplified Chinese:** choose the interface language in the top bar. Native menus, tool dialogs and built-in writing templates are localized. Existing document text is never translated by switching the interface.
- **AI assistance:** revise selected text, translate documents, preview formatting suggestions and create presentation outlines. Review before applying; ordinary edits support undo. Choose Codex or WorkBuddy directly in the chat sidebar, or connect your own compatible API.
- **Presentations:** six original content structures, twelve original editable PPTX starter decks (English and Chinese), and three color themes. Import your own PPTX, edit text and slides, preserve existing slide objects, and export PPTX or images.
- **PDF and images:** organize, rotate, merge and split PDF pages; add annotations; extract text or use OCR; crop, resize, rotate and adjust images; keep reusable assets in a local library.
- **Storage:** local folders, iCloud Drive, WebDAV and configured rclone remotes. Connect Google Drive, Dropbox, NAS, Baidu or Quark through a supported provider. Write access depends on the remote driver and your permissions.

## Quick start

1. Download `Rust-Word-0.4.4-macOS-arm64.zip` from [Releases](https://github.com/haiy/rust-word-editor/releases/latest), and compare its SHA-256 with `SHA256SUMS.txt`.
2. Unzip and open **Rust Word.app**. Save and quit an older version before updating.
3. Open a local `.docx` or UTF-8 `.txt`, or drag one into the editor. Import progress and errors are shown in the window. Browser preview uses its own file picker; dropped/uploaded files save as a copy. In the browser, Save and Save As download an editable DOCX copy; a local recovery draft is retained.
4. Start a document or choose a template. Save as `.docx` to continue editing in other word processors.
4. Use **English / 中文** in the top bar to select a language. The app initially follows your system language (Chinese or English).

The downloadable app includes the PDF/OCR helper, rclone and LibreOffice rendering components. It is locally signed, **not Apple-notarized**. On a Gatekeeper prompt, use macOS Privacy & Security to review the app; do not disable system protection globally.

AI is optional and requires your own service/account. Normal document editing works offline. No API keys, account tokens, cookies, drive configuration or private research templates are distributed with the project.

See [Cloud storage setup](CLOUD_STORAGE.md) for Google Drive / Dropbox authorization, shared folders and network settings.

## AI conversations

Open **AI Assistant → Chat** and type a request without selecting document text. Continue the conversation, use **Reference selection / Reference document** to attach text explicitly, then copy an answer or insert it at the end of the document. Referenced selections can be replaced while the original text is still unchanged; insertion and replacement support Undo. **Rewrite selection** keeps the previous preview-and-apply workflow.

The chat sidebar has a direct Codex / WorkBuddy / GPU / compatible API selector. Switching services keeps the conversation and input draft; replies show the service and, for WorkBuddy, the actual model. Codex reuses its local login. WorkBuddy uses the CLI bundled with the installed macOS app and an existing custom model from its local model settings. Choose that model in AI settings; the default is the first configured custom model. Word creates a private temporary CLI configuration and does not import WorkBuddy projects, memory, plugins or hooks. WorkBuddy subscription-only desktop login is not supported by this adapter; a configured custom model with a valid endpoint/key is required. No request silently falls back to another provider. Each conversation supports up to 20 turns. Closing the sidebar or changing the interface language retains the chat and draft; restarting the app clears chat history. Documents are only included when you attach a reference, and AI answers modify the document only when you apply them.

## Build

On macOS with Rust, Node.js, Python 3.11+ and Xcode command-line tools:

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

All original starter assets are included in the repository. Packaging, optional external programs and integration testing are documented in [BUILD.md](BUILD.md).

## Scope

This is a working desktop editor, not a complete replacement for every Microsoft Office feature. Complex external DOCX files may require saving a copy to protect unsupported objects. Legacy `.doc`, macros, full tracked changes and exact pagination parity are not supported. PDF-to-Word reconstructs a text flow rather than reproducing arbitrary layouts. PPTX editing preserves existing artwork but does not expose every shape property. Image export of imported PPTX requires LibreOffice. Some Quark drivers are read-only.

Changing the interface language preserves the current Word/PPT/PDF/image workspace, but restarts the UI and clears undo history and unapplied AI previews. The confirmation dialog explains this before switching.

## Architecture

- `src/`: Rust native window, document codecs, storage, media and AI services.
- `ui/`: Tiptap/ProseMirror editing interface, bundled locally without a CDN.
- `locales/en.json`: English UI and service-message catalog; the Chinese source is the reference language.
- `scripts/`: portable build, packaging and original-template generation tools.
- `tests/`: document interoperability, service contracts and UI checks.

Document editing does not run a production HTTP server. `--rpc` and `--test-server` are isolated development interfaces; use a disposable directory and bind test servers to loopback.

## License

The project and original starter templates are available under the [MIT License](LICENSE). Third-party components retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). No affiliation with Microsoft or WPS is implied.
