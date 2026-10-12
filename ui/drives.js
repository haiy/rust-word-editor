const $ = selector => document.querySelector(selector);
const node = (tag, value, className) => { const el = document.createElement(tag); if (value !== undefined) el.textContent = value; if (className) el.className = className; return el; };
export function installDrives({ editor, actions, invoke, modal, guard, loadDocument, toast, documentName }) {
  const legacyCloud = actions.cloud;
  const panel = node('div'); panel.id = 'drive-overlay'; panel.className = 'drive-overlay hidden';
  panel.innerHTML = '<section class="drive-dialog" role="dialog" aria-modal="true" aria-label="个人云空间"><header><div><h2>个人云空间</h2><p>百度、夸克与 NAS 统一管理。复用已有授权，文件按需下载，Word 始终保留本机编辑副本。</p></div><button id="drive-close" aria-label="关闭个人云空间">×</button></header><div class="drive-layout"><aside><div id="drive-connections"></div><button id="drive-add">＋ 添加网盘 / NAS</button><button id="drive-webdav">iCloud / 同步文件夹 / WebDAV</button><button id="drive-refresh-connections">检测已有连接</button></aside><main><div class="drive-toolbar"><button id="drive-parent">上一级</button><button id="drive-refresh">刷新文件</button><button id="drive-upload-word" class="primary">上传当前 Word</button><button id="drive-upload-file">上传文件</button><button id="drive-remove">移除连接</button><input id="drive-upload-input" type="file" accept=".docx,.pptx,.pdf,.png,.jpg,.jpeg,.webp,.txt" hidden></div><p id="drive-path"></p><div id="drive-list"></div></main></div><footer id="drive-status" role="status"></footer></section>';
  document.body.append(panel);
  let connections = [], discovery = null, active = null, path = '', busy = false;
  const status = (message, error = false) => { $('#drive-status').textContent = message; $('#drive-status').classList.toggle('error', error); };
  const run = async fn => { if (busy) return; busy = true; panel.classList.add('busy'); status('正在连接或传输…'); try { await fn(); } catch (error) { status(error.message, true); } finally { busy = false; panel.classList.remove('busy'); } };
  const wire = (selector, fn) => { $(selector).onclick = () => run(fn); };
  const label = c => ({ baidu: '百度网盘', quark: '夸克网盘', nas: 'NAS', rclone: '其他网盘' }[c.provider] || c.provider);
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
      const download = node('button', '下载'); download.onclick = () => run(async () => { const r = await invoke('driveDownload', { connectionId: active.id, path: remotePath }); await window.rustWord.suite.save(r.data, r.name); }); row.append(download);
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
    const wait = modal('添加网盘 / NAS', '<label>来源<select id="drive-provider"><option value="baidu">百度网盘</option><option value="quark">夸克网盘</option><option value="nas">NAS</option><option value="rclone">其他已授权网盘</option></select></label><label>已有客户端连接<select id="drive-remote-select"></select></label><label>子目录<input id="drive-remote-path" placeholder="例如：baidu、quark、nas"></label><label>工作文件夹<input id="drive-folder" value="RustWord" placeholder="留空可浏览现有根目录"></label><label class="setting-check"><input id="drive-readonly" type="checkbox">只读连接</label><label class="setting-check"><input id="drive-create" type="checkbox" checked>创建工作文件夹（仅在有写入权限时）</label><p id="drive-connect-hint"></p>', [['取消', 'cancel'], ['连接', 'connect', true]]);
    const remotes = discovery.remotes || []; for (const remote of remotes) { const option = node('option', remote.name); option.value = remote.name; $('#drive-remote-select').append(option); }
    const update = () => { const provider = $('#drive-provider').value; const preset = discovery.presets?.find(p => p.provider === provider); $('#drive-readonly').checked = provider === 'quark'; $('#drive-create').checked = provider !== 'quark'; $('#drive-folder').value = provider === 'quark' ? '' : 'RustWord'; if (preset) { const split = preset.remote.indexOf(':'); const name = preset.remote.slice(0, split + 1); if ([...$('#drive-remote-select').options].some(o => o.value === name)) $('#drive-remote-select').value = name; else if ([...$('#drive-remote-select').options].some(o => o.value === name.slice(0, -1))) $('#drive-remote-select').value = name.slice(0, -1); $('#drive-remote-path').value = preset.remote.slice(split + 1); $('#drive-readonly').checked = !!preset.readonlyRecommended; $('#drive-folder').value = preset.folder ?? 'RustWord'; $('#drive-create').checked = !preset.readonlyRecommended; } $('#drive-connect-hint').textContent = provider === 'quark' ? '夸克部分已有驱动只支持读取。连接与上传结果会以服务实际响应为准。' : '复用客户端已保存的账号授权，本应用不复制网盘 token 或 Cookie。'; };
    $('#drive-provider').onchange = () => { update(); if ($('#drive-provider').value === 'rclone') { $('#drive-remote-path').value = ''; $('#drive-folder').value = 'RustWord'; $('#drive-readonly').checked = false; $('#drive-create').checked = true; } }; update();
    if (await wait !== 'connect') return;
    let remote = $('#drive-remote-select').value; if (!remote) throw new Error('没有可复用的网盘授权。先在 rclone / 盘神中连接账号，或使用同步文件夹 / WebDAV。'); if (!remote.endsWith(':')) remote += ':';
    const sub = $('#drive-remote-path').value.trim().replace(/^\/+|\/+$/g, ''); remote += sub;
    const result = await invoke('driveConnect', { provider: $('#drive-provider').value, remote, folder: $('#drive-folder').value.trim(), readonly: $('#drive-readonly').checked, createFolder: $('#drive-create').checked });
    await discover(); active = result.connection; path = ''; renderConnections(); await list(); status(result.status || '已连接');
  }
  actions.cloud = actions.drives = () => run(async () => { panel.classList.remove('hidden'); await discover(); if (active) await list(); else $('#drive-list').replaceChildren(node('p', '选择百度、夸克、NAS 或其他已授权的文件位置。')); });
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
