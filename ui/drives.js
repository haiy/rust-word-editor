const $ = selector => document.querySelector(selector);
const node = (tag, value, className) => { const el = document.createElement(tag); if (value !== undefined) el.textContent = value; if (className) el.className = className; return el; };
export function installDrives({ editor, actions, invoke, modal, guard, loadDocument, toast, documentName }) {
  const legacyCloud = actions.cloud;
  const panel = node('div'); panel.id = 'drive-overlay'; panel.className = 'drive-overlay hidden';
  panel.innerHTML = '<section class="drive-dialog" role="dialog" aria-modal="true" aria-label="个人云空间"><header><div><h2>个人云空间</h2><p>Google Drive、Dropbox、百度、夸克与 NAS 统一管理。文件按需下载，Word 保留本机编辑副本。</p></div><button id="drive-close" aria-label="关闭个人云空间">×</button></header><div class="drive-layout"><aside><div id="drive-connections"></div><button id="drive-add">＋ 添加网盘 / NAS</button><button id="drive-webdav">iCloud / 同步文件夹 / WebDAV</button><button id="drive-refresh-connections">检测已有连接</button></aside><main><div class="drive-toolbar"><button id="drive-parent">上一级</button><button id="drive-refresh">刷新文件</button><button id="drive-upload-word" class="primary">上传当前 Word</button><button id="drive-upload-file">上传文件</button><button id="drive-remove">移除连接</button><input id="drive-upload-input" type="file" accept=".docx,.pptx,.pdf,.png,.jpg,.jpeg,.webp,.txt" hidden></div><p id="drive-path"></p><div id="drive-list"></div></main></div><footer id="drive-status" role="status"></footer></section>';
  document.body.append(panel);
  let connections = [], discovery = null, active = null, path = '', busy = false;
  const status = (message, error = false) => { $('#drive-status').textContent = message; $('#drive-status').classList.toggle('error', error); };
  const run = async fn => { if (busy) return; busy = true; panel.classList.add('busy'); status('正在连接或传输…'); try { await fn(); } catch (error) { status(error.message, true); } finally { busy = false; panel.classList.remove('busy'); } };
  const wire = (selector, fn) => { $(selector).onclick = () => run(fn); };
  const label = c => ({ baidu: '百度网盘', quark: '夸克网盘', nas: 'NAS', 'google-drive': 'Google Drive', dropbox: 'Dropbox', rclone: '其他网盘' }[c.provider] || c.provider);
  function renderConnections() {
    $('#drive-connections').replaceChildren();
    for (const connection of connections) { const button = node('button', label(connection)); button.className = 'drive-connection' + (active?.id === connection.id ? ' active' : ''); const details = node('small', `${connection.remote}/${connection.folder || ''}${connection.readonly ? ' · 只读' : ''}`); button.append(details); button.onclick = () => run(async () => { active = connection; path = ''; renderConnections(); await list(); }); $('#drive-connections').append(button); }
    if (!connections.length) $('#drive-connections').append(node('p', '还没有连接。选择“添加网盘 / NAS”即可接入。', 'drive-empty'));
  }
  async function discover() { discovery = await invoke('driveDiscover'); connections = discovery.connections || []; if (active) active = connections.find(c => c.id === active.id) || null; renderConnections(); status(discovery.available ? `已检测已有客户端授权：${(discovery.remotes || []).length} 个远端。连接容量以实际网盘为准。` : '未检测到可用 rclone 授权。可先使用同步文件夹或 WebDAV，或在客户端配置网盘后重新检测。'); }
  async function list() {
    if (!active) { $('#drive-list').replaceChildren(node('p', '先选择一个连接，或添加新的网盘位置。')); return; }
    const result = await invoke('driveList', { connectionId: active.id, path });
    $('#drive-path').textContent = `${label(active)} / ${active.folder || ''}${path ? '/' + path : ''}`;
    $('#drive-list').replaceChildren();
    const directories = result.directories || [], files = result.files || [];
    for (const directory of directories) { const button = node('button', `▸ ${directory.name}`, 'drive-folder'); button.onclick = () => run(async () => { path = directory.path || [path, directory.name].filter(Boolean).join('/'); await list(); }); $('#drive-list').append(button); }
    for (const file of files) {
      const row = node('div', undefined, 'drive-file'); row.append(node('span', file.name), node('small', `${((file.bytes || 0) / 1024).toFixed(1)} KB`));
      const remotePath = file.path || [path, file.name].filter(Boolean).join('/');
      const download = node('button', '下载'); download.onclick = () => run(async () => { const r = await invoke('driveDownload', { connectionId: active.id, path: remotePath }); const saved = await window.rustWord.suite.save(r.data, r.name); status(saved ? `已导出 ${r.name}` : '已取消'); }); row.append(download);
      if (/\.docx$/i.test(file.name)) { const open = node('button', '打开 Word 副本'); open.onclick = () => run(async () => { const r = await invoke('driveDownload', { connectionId: active.id, path: remotePath }); const document = await invoke('importDocument', r); if (!await guard()) { status('已取消打开'); return; } await invoke('new'); loadDocument(document); window.rustWord.markTemplateDirty(); status('已打开网盘 Word 的本机副本。编辑后可再次上传。'); }); row.append(open); }
      if (/\.pdf$/i.test(file.name)) { const open = node('button', 'PDF 工具'); open.onclick = () => run(async () => { const r = await invoke('driveDownload', { connectionId: active.id, path: remotePath }); panel.classList.add('hidden'); actions.pdfTools(); await window.rustWord.suite.setPdf(r); }); row.append(open); }
      $('#drive-list').append(row);
    }
    if (!directories.length && !files.length) $('#drive-list').append(node('p', '当前文件夹为空。可上传 Word、PPT、PDF 或图片。'));
    $('#drive-upload-word').disabled = !!result.readonly; $('#drive-upload-file').disabled = !!result.readonly;
    status(result.status || (result.readonly ? '已连接，只读。当前驱动不支持写入。' : '目录已读取。上传时会核验实际写入结果。'));
  }
  async function add() {
    if (!discovery) await discover();
    const wait = modal('添加网盘 / NAS', '<label>来源<select id="drive-provider"><option value="google-drive">Google Drive</option><option value="dropbox">Dropbox</option><option value="baidu">百度网盘</option><option value="quark">夸克网盘</option><option value="nas">NAS</option><option value="rclone">其他已授权网盘</option></select></label><label>已有客户端连接<select id="drive-remote-select"></select></label><label>子目录<input id="drive-remote-path" placeholder="例如：Documents 或留空"></label><label>工作文件夹<input id="drive-folder" value="RustWord" placeholder="留空可浏览现有根目录"></label><label class="setting-check"><input id="drive-readonly" type="checkbox">只读连接</label><label class="setting-check"><input id="drive-create" type="checkbox" checked>创建工作文件夹（仅在有写入权限时）</label><p id="drive-connect-hint" role="status"></p><button type="button" id="drive-refresh-auth">授权完成，刷新连接</button><details id="drive-setup"><summary>首次连接 / 授权帮助</summary><ol><li>在终端运行下方命令，输入 n 新建连接，并为它取名。</li><li id="drive-setup-type"></li><li>按提示在浏览器登录并授权，保存配置后返回这里刷新。</li></ol><input id="drive-setup-command" readonly aria-label="授权命令"><p id="drive-setup-note"></p><p>如果电脑已安装网盘同步客户端，也可使用“iCloud / 同步文件夹 / WebDAV”选择本机同步目录。</p></details>', [['取消', 'cancel'], ['连接', 'connect', true]]);
    const providerBackend = { 'google-drive': 'drive', dropbox: 'dropbox' };
    const renderRemotes = () => {
      const provider = $('#drive-provider').value, backend = providerBackend[provider];
      const select = $('#drive-remote-select'), previous = select.value;
      select.replaceChildren();
      for (const remote of discovery.remotes || []) {
        if (backend && remote.type !== backend) continue;
        const option = node('option', `${remote.name} (${remote.type})`);
        option.value = remote.name; select.append(option);
      }
      if ([...select.options].some(option => option.value === previous)) select.value = previous;
      const empty = !select.options.length;
      if (empty) { const option = node('option', '尚未发现匹配的账号授权'); option.value = ''; select.append(option); }
      select.disabled = empty;
      $('#modal-actions button:last-child').disabled = empty;
      $('#drive-setup').open = empty;
      $('#drive-setup-command').value = discovery.setupCommand || 'rclone config';
      $('#drive-setup-type').textContent = provider === 'google-drive'
        ? '存储类型选择 drive；需要读写已有文件时，scope 选择 drive。'
        : provider === 'dropbox' ? '存储类型选择 dropbox，按提示完成 Dropbox 授权。'
        : '选择对应网盘类型；NAS 可配置 WebDAV、SFTP 或 SMB。';
      $('#drive-setup-note').textContent = provider === 'google-drive'
        ? 'Google Drive 建议配置自己的 OAuth 客户端 ID。原生 Google 文档会导出为 Office 副本；上传的 DOCX 保持文件格式。'
        : '授权保存在本机 rclone 配置中。配置为只读的账号无法上传，移除本应用连接不会撤销账号授权。';
      $('#drive-connect-hint').textContent = empty ? '请先完成账号授权，再刷新连接。已通过别名或网关接入的账号可选“其他已授权网盘”。'
        : provider === 'quark' ? '夸克部分已有驱动只支持读取。连接与上传结果会以服务实际响应为准。'
        : '仅显示匹配的已配置连接。连接时会实际读取目录，上传后回读校验。';
    };
    const update = () => {
      const provider = $('#drive-provider').value;
      $('#drive-remote-path').value = '';
      $('#drive-readonly').checked = provider === 'quark';
      $('#drive-create').checked = provider !== 'quark';
      $('#drive-create').disabled = provider === 'quark';
      $('#drive-folder').value = provider === 'quark' ? '' : 'RustWord';
      renderRemotes();
    };
    $('#drive-provider').onchange = update;
    $('#drive-readonly').onchange = () => {
      $('#drive-create').disabled = $('#drive-readonly').checked;
      $('#drive-create').checked = !$('#drive-readonly').checked;
    };
    $('#drive-setup-command').onclick = event => event.target.select();
    $('#drive-refresh-auth').onclick = async () => {
      const button = $('#drive-refresh-auth');
      button.disabled = true;
      $('#modal-actions button:last-child').disabled = true;
      try {
        discovery = await invoke('driveDiscover');
        // The modal may have been dismissed while discovery was pending.
        if (!$('#modal').classList.contains('hidden') && $('#drive-refresh-auth') === button) renderRemotes();
      } catch (error) {
        if ($('#drive-refresh-auth') === button) $('#drive-connect-hint').textContent = error.message;
      } finally { button.disabled = false; }
    };
    update();
    if (await wait !== 'connect') return;
    let remote = $('#drive-remote-select').value; if (!remote) throw new Error('没有可复用的网盘授权。先在 rclone / 盘神中连接账号，或使用同步文件夹 / WebDAV。'); if (!remote.endsWith(':')) remote += ':';
    const sub = $('#drive-remote-path').value.trim().replace(/^\/+|\/+$/g, ''); remote += sub;
    const result = await invoke('driveConnect', { provider: $('#drive-provider').value, remote, folder: $('#drive-folder').value.trim(), readonly: $('#drive-readonly').checked, createFolder: $('#drive-create').checked });
    await discover(); active = result.connection; path = ''; renderConnections(); await list(); status(result.status || '已连接');
  }
  actions.cloud = actions.drives = () => run(async () => { panel.classList.remove('hidden'); await discover(); if (active) await list(); else $('#drive-list').replaceChildren(node('p', '选择 Google Drive、Dropbox、百度、夸克、NAS 或其他已授权的文件位置。')); });
  wire('#drive-add', add); wire('#drive-refresh-connections', discover); wire('#drive-refresh', list);
  wire('#drive-parent', async () => { path = path.split('/').slice(0, -1).join('/'); await list(); });
  wire('#drive-upload-word', async () => { if (!active) throw new Error('请先连接网盘'); const r = await invoke('exportDocument', { document: editor.getJSON(), name: documentName() }); const saved = await invoke('driveUpload', { connectionId: active.id, path, name: r.name, data: r.data }); await list(); status(saved.status || `已上传 ${saved.name}`); });
  wire('#drive-upload-file', async () => { if (!active) throw new Error('请先连接网盘'); const file = await window.rustWord.suite.pick('file', '#drive-upload-input'); if (!file) return; const ext = file.name.split('.').pop(); const name = file.name.replace(/\.[^.]+$/, '') + '-' + Date.now() + '.' + ext; const saved = await invoke('driveUpload', { connectionId: active.id, path, name, data: file.data }); await list(); status(saved.status || `已上传 ${saved.name}`); });
  wire('#drive-remove', async () => { if (!active) return; const answer = await modal('移除这个连接？', '<p>仅从 Rust Word 移除连接，不删除网盘文件或客户端授权。</p>', [['取消', 'cancel'], ['移除', 'remove', true]]); if (answer !== 'remove') return; await invoke('driveDisconnect', { connectionId: active.id }); active = null; path = ''; await discover(); $('#drive-list').replaceChildren(); });
  $('#drive-webdav').onclick = () => { if (busy) return; panel.classList.add('hidden'); legacyCloud().catch(e => toast(e.message)); };
  $('#drive-close').onclick = () => { if (!busy) panel.classList.add('hidden'); };
  document.addEventListener('keydown', e => { if (e.key === 'Escape' && !busy && $('#modal').classList.contains('hidden')) panel.classList.add('hidden'); });
  window.rustWord.drives = { discover, list, add };
}
