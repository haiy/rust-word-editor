# Security and privacy

[English](SECURITY.md) · [简体中文](docs/SECURITY.zh-CN.md)

Do not commit API keys, cookies, access tokens, rclone configuration, private documents or local application state. The public repository has no built-in accounts or cloud capacity.

Settings stay under `~/Library/Application Support/Rust Word/`. Secret-bearing files are written with owner-only permissions. The webview receives only a flag indicating whether an API key/password is configured; stored secrets are not returned to the UI. Existing remote credentials remain in the user's rclone configuration.

AI requests send the explicitly selected text, document blocks or presentation source to the configured provider when the user starts the corresponding operation. Review the provider and document content before sending. Local editing requires no model service.

The public distribution includes original templates only. Import external documents and templates only when you have permission to use them. Macro execution is disabled in the rendering helper.

`--test-server` is a development-only unauthenticated interface. Bind it to loopback and use a disposable directory. Never expose it to the internet.

Please report vulnerabilities using the repository's private security reporting feature rather than publishing credentials or private documents in an issue. If a credential is exposed, revoke it with its provider; deleting a file is not sufficient.
