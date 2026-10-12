# Cloud storage

[English](CLOUD_STORAGE.md) · [简体中文](CLOUD_STORAGE.zh-CN.md)

Rust Word supports Google Drive and Dropbox through rclone, alongside NAS,
Baidu, Quark, WebDAV and local sync folders. The macOS download bundles rclone.
Authorization belongs to your own account and stays in your local rclone
configuration. Rust Word stores connection names and paths, not OAuth tokens.

## Connect an account

1. Open **Tools → Personal cloud storage → Add cloud drive / NAS**, then choose **Google Drive**
   or **Dropbox**.
2. Expand **First connection / authorization help**. Copy the command into
   Terminal. It uses the rclone bundled with your app; `rclone config` also works
   if rclone is installed separately. Both must use the same configuration file.
3. Create a named remote with `n`. Choose storage type `drive` for Google Drive,
   or `dropbox` for Dropbox. Complete browser sign-in and consent, and save.
4. Return to Rust Word and choose **Refresh authorized accounts**. Only remotes
   with the matching backend type appear. Pick your account.
5. Choose an optional subdirectory and work folder. `RustWord` is the default;
   clear the work folder to browse the account root. Choose read-only access if
   you only need to browse/download. Connecting reads the actual directory.

For Google Drive, configure your own OAuth client ID following the
[official setup guide](https://rclone.org/drive/#making-your-own-client-id).
For existing-file read/write access use the `drive` scope; `drive.readonly`
permits reading only, and `drive.file` sees files created by rclone.
Shared Drives can be configured during rclone setup. A restricted root folder
can be set in that remote's advanced settings.

Dropbox personal accounts use the [standard browser authorization flow](https://rclone.org/dropbox/).
Team/shared-folder access depends on the account and remote configuration.
Aliases, encrypted remotes and gateways should use **Other authorized cloud drives**.

## Work with files

- Browse folders, download files, upload the current Word document or choose a
  local file. **Open Word copy** downloads DOCX into the editor for local editing.
- Uploads create new files and refuse existing names. Save under a new name to
  upload a revision. Successful uploads are downloaded again and checked with
  SHA-256; the UI reports when verification cannot complete.
- Google-native documents appear as Office exports (DOCX/XLSX/PPTX where
  supported). DOCX uploads remain ordinary files, with automatic conversion to
  Google-native documents disabled. There is no collaborative Google Docs editor.
- Individual transfers are limited to 96 MiB. Account storage, sharing rights
  and provider limits still apply. Removing a Rust Word connection leaves files
  and rclone authorization intact.

## Network and troubleshooting

Rclone inherits `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` from the app process.
Use `NO_PROXY` for local NAS hosts when needed. Finder-launched apps may not
inherit Terminal environment variables; launch the app executable from the
configured Terminal if your network requires a proxy. No proxy or account is
preconfigured in the release.

If no account appears, finish rclone setup and refresh. Reauthorize expired
accounts in rclone. A read-only remote cannot upload even if the app's read-only
checkbox is cleared. If you already use the Google Drive or Dropbox desktop
sync app, you can instead select its local folder using **iCloud / Synced folder /
WebDAV**; the external sync app handles synchronization.

## Verification scope

`python3 tests/drive-providers.test.py` uses synthetic provider metadata and
real rclone transfers to disposable local storage. It verifies provider
matching, directory browsing, editable DOCX round trips, readback hashes,
read-only behavior and existing-file protection without personal credentials.
It does **not** verify Google/Dropbox OAuth or their live APIs. Those require
an authorized account and reachable provider services.
