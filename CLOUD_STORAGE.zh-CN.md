# 云存储接入

[English](CLOUD_STORAGE.md) · [简体中文](CLOUD_STORAGE.zh-CN.md)

Rust Word 通过 rclone 支持 Google Drive、Dropbox，也可连接 NAS、百度、夸克、WebDAV 和本机同步目录。macOS 安装包已内置 rclone。账号授权保存在本机 rclone 配置；应用只保存连接名称和路径，不复制 OAuth 令牌。

## 首次连接

1. 打开 **工具 → 个人云空间 → 添加网盘 / NAS**，选择 **Google Drive** 或 **Dropbox**。
2. 展开“首次连接 / 授权帮助”，复制命令到终端运行。命令使用应用内置 rclone；单独安装的 `rclone config` 也可用，但两者需读取同一份配置。
3. 输入 `n` 新建连接并命名。Google Drive 选择 `drive` 类型，Dropbox 选择 `dropbox`。按提示在浏览器登录、授权并保存。
4. 回到应用点“授权完成，刷新连接”，列表只显示匹配类型的账号。
5. 选择子目录和工作文件夹。默认使用 `RustWord`；工作文件夹留空可浏览账号根目录。只需浏览、下载时勾选只读。点击连接会实际读取目录。

Google Drive 建议按[官方说明](https://rclone.org/drive/#making-your-own-client-id)配置自己的 OAuth 客户端 ID。读写已有文件选择 `drive` scope；`drive.readonly` 仅可读取；`drive.file` 仅能看见 rclone 创建的文件。共享云端硬盘可在 rclone 配置中选择，也可在高级设置限制根文件夹。

Dropbox 按[官方浏览器授权流程](https://rclone.org/dropbox/)配置；团队或共享文件夹的可用权限取决于账号和远端设置。通过 alias、crypt 或网关接入的账号，选择“其他已授权网盘”。

## 文件操作

- 浏览目录、下载文件、上传当前 Word 或本地文件。“打开 Word 副本”会下载 DOCX 并在本机编辑。
- 上传拒绝覆盖同名文件；修改后请换名称上传。上传后会下载回读并检查 SHA-256；未能回读时会明确提示。
- Google 原生文档按支持的格式导出为 DOCX/XLSX/PPTX 等 Office 副本。上传 DOCX 保持普通文件，不自动转换成 Google 原生文档；不提供 Google Docs 在线协同编辑。
- 单文件传输上限 96 MiB；容量、共享权限与服务限制由账号决定。移除应用内连接不删除远端文件、不撤销 rclone 授权。

## 网络与排错

rclone 会继承应用进程的 `HTTP_PROXY`、`HTTPS_PROXY` 和 `NO_PROXY`。本地 NAS 如需绕过代理，请配置 `NO_PROXY`。Finder 启动的应用可能不继承终端环境；需要代理时可从已配置环境的终端运行应用可执行文件。公开版不预置代理或账号。

账号未出现时先完成 rclone 授权并刷新；授权过期需在 rclone 重新登录。服务端只读账号不会因取消应用的只读勾选而获得写权限。已安装 Google Drive / Dropbox 桌面同步客户端时，也可通过“iCloud / 同步文件夹 / WebDAV”选择本机同步目录，由客户端负责同步。

## 验证范围

`python3 tests/drive-providers.test.py` 使用合成账号类型信息和真实 rclone 对临时本地目录读写，验证账号匹配、目录浏览、可编辑 DOCX 往返、回读校验、只读与同名保护，不读取个人凭据。该测试不代表 Google / Dropbox 的真实 OAuth 和在线 API 验证；后者需要账号授权及可达的服务。
