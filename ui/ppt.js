// Editable presentations, kept separate from the open Word document.
const $ = selector => document.querySelector(selector);
const clone = value => JSON.parse(JSON.stringify(value));
const xml = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' }[c]));
const uid = () => `slide-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
const makeSlide = (title, body = [], layout = 'content') => ({ id: uid(), title, body, layout });
export const presentationTemplates = [
  { id: 'work', name: '工作汇报', category: '汇报', description: '成果、进度、问题与下一步', slides: [['工作汇报', ['汇报人 · 团队 · 日期']], ['本期成果', ['完成事项与实际交付物', '关键进展和对业务的影响']], ['问题与支持', ['当前阻碍与已经尝试的办法', '需要的资源与明确请求']], ['下一步计划', ['优先事项与负责人', '完成时间与验收标准']]] },
  { id: 'project', name: '项目计划', category: '计划', description: '目标、范围、里程碑与验收', slides: [['项目计划', ['项目名称 · 负责人 · 日期']], ['背景与目标', ['当前需要解决的问题', '可衡量的目标与受益者']], ['实施安排', ['工作范围与关键里程碑', '人员、时间和资源安排']], ['风险与验收', ['主要风险与应对办法', '完成条件与验收方式']]] },
  { id: 'analysis', name: '分析报告', category: '汇报', description: '结论、依据与行动建议', slides: [['分析报告', ['分析主题 · 时间范围 · 编写人']], ['主要发现', ['用一段事实说明主要发现', '注明数据来源和指标口径']], ['原因与影响', ['解释差异，区分事实与假设', '说明影响范围和待验证事项']], ['行动建议', ['具体行动与预期结果', '负责人、完成时间和验证方法']]] },
  { id: 'training', name: '培训课程', category: '教学', description: '学习目标、知识、练习与总结', slides: [['培训课程', ['课程主题 · 讲师 · 日期']], ['学习目标', ['完成后能够独立执行的任务', '学习内容与适用场景']], ['内容与练习', ['关键方法与具体示例', '练习任务与反馈标准']], ['课后行动', ['在实际工作中尝试一次', '用可观察的结果检查掌握情况']]] },
  { id: 'product', name: '产品介绍', category: '介绍', description: '用户问题、方案、价值与落地', slides: [['产品介绍', ['产品名称 · 使用场景']], ['用户的问题', ['具体用户与实际使用场景', '现有流程中的主要困难']], ['产品方案', ['核心功能与操作方式', '用一个案例展示解决过程']], ['使用与反馈', ['开始使用的步骤', '验证效果的方法与反馈渠道']]] },
  { id: 'blank', name: '空白演示', category: '通用', description: '从标题与一页正文开始', slides: [['演示标题', ['副标题或演示说明']], ['内容标题', ['第一项内容', '第二项内容']]] },
];
const fallbackTheme = { id: '', name: '清晰蓝', colors: { accent1: '235BAB', accent2: 'D7E4F4', lt1: 'FFFFFF', dk1: '172B42' }, fonts: {} };
const safeColor = (theme, key, fallback) => /^[\da-f]{6}$/i.test(theme?.colors?.[key] || '') ? `#${theme.colors[key]}` : fallback;
function splitLines(value, maxUnits) {
  const lines = []; let line = '', units = 0;
  for (const c of String(value || '')) {
    if (c === '\n') { lines.push(line); line = ''; units = 0; continue; }
    const width = /[\u0000-\u00ff]/.test(c) ? .55 : 1;
    if (units + width > maxUnits && line) { lines.push(line); line = ''; units = 0; }
    line += c; units += width;
  }
  lines.push(line); return lines;
}
export function slideSvg(slide, theme = fallbackTheme, index = 0, count = 1) {
  const cover = slide.layout === 'cover' || slide.layout === 'section';
  const accent = safeColor(theme, 'accent1', '#235BAB'), bg = cover ? accent : safeColor(theme, 'lt1', '#ffffff');
  const fg = cover ? safeColor(theme, 'lt1', '#ffffff') : safeColor(theme, 'dk1', '#172b42');
  const titleColor = cover ? fg : accent;
  const chars = (slide.body || []).join('').length;
  const size = chars > 1000 ? 13 : chars > 650 ? 16 : chars > 350 ? 19 : 22;
  const titleSize = cover ? 44 : 32;
  const titleLines = splitLines(slide.title, (cover ? 690 : 835) / titleSize);
  let bodyY = cover ? 328 : 184;
  const titleY = cover ? (slide.layout === 'section' ? 225 : 188) : 83;
  let texts = titleLines.map((line, i) => `<text x="${cover ? 64 : 62}" y="${titleY + i * titleSize * 1.25}" fill="${titleColor}" font-size="${titleSize}" font-weight="700">${xml(line)}</text>`).join('');
  for (const paragraph of slide.body || []) {
    const lines = splitLines(paragraph, (cover ? 670 : 790) / size);
    for (const [i, line] of lines.entries()) { texts += `<text x="${cover ? 68 : 92}" y="${bodyY}" fill="${fg}" font-size="${size}">${xml(line)}</text>`; if (!cover && i === 0) texts += `<circle cx="76" cy="${bodyY - 7}" r="3" fill="${accent}"/>`; bodyY += size * 1.35; }
    bodyY += 9;
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 540" role="img" aria-label="${xml(slide.title)}"><rect width="960" height="540" fill="${bg}"/>${cover ? `<rect x="800" width="160" height="540" fill="${safeColor(theme, 'accent2', '#d7e4f4')}"/>` : `<rect width="12" height="540" fill="${accent}"/><rect x="62" y="126" width="836" height="3" fill="${accent}"/>`}<g font-family="PingFang SC,Microsoft YaHei,Noto Sans CJK SC,Arial,sans-serif">${texts}<text x="64" y="518" fill="${fg}" font-size="11">Rust Word · ${String(index + 1).padStart(2, '0')} / ${String(count).padStart(2, '0')}</text></g></svg>`;
}

export function installPpt({ editor, actions, invoke, toast, modal, documentName, revision }) {
  let catalog = null, deck = null, selected = 0, selectedTemplate = presentationTemplates[0], busy = false, dirty = false, nativeVersion = 0, previewToken = 0;
  const nativeImages = new Map();
  const overlay = document.createElement('div'); overlay.id = 'ppt-overlay'; overlay.className = 'ppt-overlay hidden';
  overlay.innerHTML = '<section class="ppt-dialog" role="dialog" aria-modal="true" aria-label="PPT 工作区"><header class="ppt-header"><div><h2 id="ppt-heading">PPT 模板与编辑</h2><p id="ppt-hint">选择结构，编辑内容，导出可编辑的演示文稿。</p></div><div class="ppt-header-actions"><button id="ppt-library-button">模板库</button><button id="ppt-open">打开 PPTX</button><button id="ppt-close" aria-label="关闭 PPT 工作区">×</button></div></header><div id="ppt-main"></div><footer class="ppt-footer"><span id="ppt-status" role="status"></span><span id="ppt-format-hint">16:9 · 原生文字可编辑</span></footer><input id="ppt-file-input" type="file" accept=".pptx" hidden></section>';
  document.body.append(overlay);
  const status = (message, error = false) => { $('#ppt-status').textContent = message; $('#ppt-status').classList.toggle('error', error); };
  async function run(task) { if (busy) return; busy = true; overlay.classList.add('busy'); try { await task(); } catch (error) { status(error.message, true); toast(error.message); } finally { busy = false; overlay.classList.remove('busy'); } }
  async function themes() { if (!catalog) { catalog = await invoke('pptThemes'); if(window.__rustWordBoot.locale==='zh-CN')catalog.themes?.forEach(t=>{t.name=t.nameZh||t.name;}); } return catalog.themes?.length ? catalog.themes : [fallbackTheme]; }
  const theme = () => catalog?.themes?.find(t => t.id === deck?.themeId) || catalog?.themes?.[0] || fallbackTheme;
  async function confirmReplace() { if (!dirty) return true; const answer = await modal('保留当前 PPT 修改？', '<p>当前 PPT 还没有导出。导出后再继续，或放弃这些修改。</p>', [['取消', 'cancel'], ['放弃修改', 'discard'], ['导出 PPTX', 'save', true]]); if (answer === 'cancel') return false; if (answer === 'save') return !!await exportPptx(); return true; }
  async function close() { if (busy) { toast('正在处理，请稍候'); return; } overlay.classList.add('hidden'); }
  $('#ppt-close').onclick = close;
  document.addEventListener('keydown', event => { if (event.key === 'Escape' && $('#modal').classList.contains('hidden') && !overlay.classList.contains('hidden')) close(); });
  function normalized(value) { return { ...clone(value), title: value.title || '演示文稿', themeId: value.themeId || theme().id, slides: (value.slides || []).map((s, i) => ({ ...s, id: s.id || uid(), title: s.title || `第 ${i + 1} 页`, body: Array.isArray(s.body) ? s.body : [String(s.body || '')], layout: s.layout || (i ? 'content' : 'cover') })) }; }
  async function setDeck(value, isDirty = true) { await themes(); if (!value.slides?.length) throw new Error('演示文稿没有页面'); deck = normalized(value); selected = 0; dirty = isDirty; nativeVersion++; nativeImages.clear(); overlay.classList.remove('hidden'); renderEditor(); }
  function renderLibrary() {
    $('#ppt-heading').textContent = '从模板开始一份演示'; $('#ppt-hint').textContent = '原创模板、内容结构与主题，也可以导入你自己的 PPTX。';
    $('#ppt-main').innerHTML = '<div class="ppt-library-toolbar"><input id="ppt-search" type="search" placeholder="搜索工作汇报、项目、培训或主题"><select id="ppt-library-kind"><option value="structure">内容模板</option><option value="theme">本机主题</option><option value="original">原创 PPTX 模板</option></select></div><div class="ppt-library-body"><div id="ppt-template-grid"></div><aside class="ppt-library-preview"><h3 id="ppt-preview-title"></h3><p id="ppt-preview-description"></p><div id="ppt-template-preview"></div><button id="ppt-template-use" class="primary">使用这个模板</button><button id="ppt-template-download" hidden>下载完整原件</button></aside></div>';
    const draw = () => {
      const kind = $('#ppt-library-kind').value, query = $('#ppt-search').value.trim().toLowerCase();
      const rows = kind === 'original' ? (catalog?.templateCatalog?.templates || []).map(t => ({ ...t, original: true, category: `${t.language} · ${t.native ? `${t.slideCount || 1} 页` : '受保护旧格式'}`, description: t.native ? '原创 OOXML 模板 · 保留母版、图形与媒体，可修改原生文字。' : '受保护旧格式未转换。原始文件已完整保留，可在有权访问的原生应用中打开。' })) : kind === 'theme' ? (catalog?.themes || [fallbackTheme]).map(t => ({ id: t.id, name: t.name, category: '本机主题', description: '应用这个主题的配色与字体', theme: t, slides: presentationTemplates[0].slides })) : presentationTemplates;
      const filtered = rows.filter(t => `${t.name} ${t.category} ${t.description}`.toLowerCase().includes(query));
      const grid = $('#ppt-template-grid'); grid.replaceChildren();
      for (const row of filtered) { const button = document.createElement('button'); button.className = 'ppt-template-card'; button.dataset.pptTemplate = row.id; button.innerHTML = (row.original ? `<div class="ppt-original-card-art"><span>${row.native ? 'PPTX' : '受保护'}</span><small>${xml(row.originalName)}</small></div>` : slideSvg(makeSlide(row.slides[0][0], row.slides[0][1], 'cover'), row.theme || theme())) + `<strong>${xml(row.name)}</strong><small>${xml(row.category)}</small>`; button.onclick = () => preview(row); grid.append(button); }
      if (filtered.length) preview(filtered.find(t => t.id === selectedTemplate?.id) || filtered[0]); else { grid.textContent = '没有匹配的模板。'; $('#ppt-template-preview').replaceChildren(); $('#ppt-template-use').disabled = true; }
    };
    const preview = async row => {
      selectedTemplate = row; const token = ++previewToken;
      $('#ppt-preview-title').textContent = row.name; $('#ppt-preview-description').textContent = row.description;
      $('#ppt-template-use').disabled = false; $('#ppt-template-use').textContent = row.original && !row.native ? '下载受保护原件' : '使用这个模板';
      $('#ppt-template-download').hidden = !row.original && !row.theme; $('#ppt-template-download').textContent = row.theme ? '下载完整 THMX 主题' : '下载完整原件';
      if (!row.original) { $('#ppt-template-preview').innerHTML = slideSvg(makeSlide(row.slides[0][0], row.slides[0][1], 'cover'), row.theme || theme()); return; }
      $('#ppt-template-preview').innerHTML = '<div class="ppt-original-preview-note">保留完整背景与对象。使用后可查看真实逐页预览。</div>';
      try { const result = await invoke('pptTemplatePreview', { templateId: row.id }); if (token !== previewToken || !$('#ppt-template-preview')) return; if (result.data) { const img = document.createElement('img'); img.src = result.data; img.alt = `${row.name} 原包缩略图`; $('#ppt-template-preview').replaceChildren(img); } } catch (error) { if (token === previewToken) status(error.message, true); }
    };
    $('#ppt-search').oninput = draw; $('#ppt-library-kind').onchange = draw;
    $('#ppt-template-download').onclick = () => run(async () => { const result = await invoke('pptTemplateFile', { templateId: selectedTemplate.id, original: true }); await window.rustWord.suite.save(result.data, result.name); status('已下载完整随包原件'); });
    $('#ppt-template-use').onclick = () => run(async () => {
      const row = selectedTemplate;
      if (row.original && !row.native) { const result = await invoke('pptTemplateFile', { templateId: row.id, original: true }); await window.rustWord.suite.save(result.data, result.name); status('已保留受保护原件；没有转换或解密'); return; }
      if (!await confirmReplace()) return;
      if (row.original) { await setDeck(await invoke('pptTemplateOpen', { templateId: row.id })); status('已打开原版页面。修改右侧文字槽，刷新预览后导出。'); }
      else { await setDeck({ title: row.name, themeId: row.theme?.id || theme().id, slides: row.slides.map(([title, body], i) => makeSlide(title, body, i ? 'content' : 'cover')) }); status('已创建可编辑页面。编辑标题和正文，再导出。'); }
    });
    draw(); status(`${presentationTemplates.length} 个原创内容模板 · ${catalog?.themes?.length || 1} 个主题 · ${catalog?.templateCatalog?.nativeCount || 0} 个原版可用模板 · 5 个受保护原件`);
  }
  async function openLibrary() { await themes(); overlay.classList.remove('hidden'); renderLibrary(); }
  function renderEditor() {
    $('#ppt-heading').textContent = deck.title; $('#ppt-hint').textContent = deck.native ? '原版背景与对象保留。修改文字槽，刷新真实预览；复制、排序或删除页面后导出。' : '选择页面修改文字；调整顺序后导出，PPTX 中仍可编辑。';
    $('#ppt-format-hint').textContent = deck.native ? `${Math.round(deck.width / 12700)}×${Math.round(deck.height / 12700)} pt · 保留原版对象` : '16:9 · 原生文字可编辑';
    $('#ppt-main').innerHTML = '<div class="ppt-editor-toolbar"><button id="ppt-add">＋ 添加页面</button><button id="ppt-delete">删除页面</button><button id="ppt-up">上移</button><button id="ppt-down">下移</button><label>主题<select id="ppt-theme"></select></label><button id="ppt-refresh-preview" hidden>刷新真实预览</button><button id="ppt-images">转为逐页图片</button><button id="ppt-export" class="primary">导出 PPTX</button></div><div class="ppt-editor-body"><nav id="ppt-slide-list" aria-label="幻灯片页面"></nav><div class="ppt-stage"><div id="ppt-slide-preview"></div><div id="ppt-page-indicator"></div></div><aside class="ppt-fields"><label>演示名称<input id="ppt-deck-title" maxlength="160"></label><label>页面标题<input id="ppt-slide-title" maxlength="160"></label><label>版式<select id="ppt-layout"><option value="cover">封面</option><option value="content">正文</option><option value="section">章节</option></select></label><label>正文（每行一项）<textarea id="ppt-slide-body" rows="10" maxlength="1800"></textarea></label><div id="ppt-native-slots" hidden></div><p id="ppt-editing-note">文字保留为原生对象。较长内容建议拆成多页。</p></aside></div>';
    for (const t of catalog?.themes || [fallbackTheme]) { const option = document.createElement('option'); option.value = t.id; option.textContent = t.name; $('#ppt-theme').append(option); }
    $('#ppt-theme').value = theme().id; $('#ppt-deck-title').value = deck.title;
    $('#ppt-theme').onchange = () => { deck.themeId = $('#ppt-theme').value; dirty = true; renderThumbnails(); previewSlide(); };
    $('#ppt-deck-title').oninput = () => { deck.title = $('#ppt-deck-title').value; $('#ppt-heading').textContent = deck.title; dirty = true; };
    $('#ppt-slide-title').oninput = () => { deck.slides[selected].title = $('#ppt-slide-title').value; dirty = true; renderThumbnails(); previewSlide(); };
    $('#ppt-slide-body').oninput = () => { deck.slides[selected].body = $('#ppt-slide-body').value.split('\n').filter(t => t.trim()); dirty = true; renderThumbnails(); previewSlide(); };
    $('#ppt-layout').onchange = () => { deck.slides[selected].layout = $('#ppt-layout').value; dirty = true; renderThumbnails(); previewSlide(); };
    $('#ppt-add').onclick = () => { if (deck.slides.length >= 50) return toast('最多 50 页'); const slide = deck.native ? { ...clone(deck.slides[selected]), id: uid() } : makeSlide('新页面', ['输入本页内容']); deck.slides.splice(selected + 1, 0, slide); selected++; dirty = true; nativeVersion++; renderEditor(); };
    $('#ppt-delete').onclick = () => { if (deck.slides.length <= 1) return toast('至少保留一页'); deck.slides.splice(selected, 1); selected = Math.min(selected, deck.slides.length - 1); dirty = true; nativeVersion++; renderEditor(); };
    const move = direction => { const next = selected + direction; if (next < 0 || next >= deck.slides.length) return; [deck.slides[selected], deck.slides[next]] = [deck.slides[next], deck.slides[selected]]; selected = next; dirty = true; nativeVersion++; renderEditor(); };
    $('#ppt-up').onclick = () => move(-1); $('#ppt-down').onclick = () => move(1);
    $('#ppt-export').onclick = () => run(exportPptx); $('#ppt-images').onclick = () => run(exportImages);
    if (deck.native) { $('#ppt-add').textContent = '＋ 复制当前页'; $('#ppt-theme').disabled = true; $('#ppt-theme').title = '原版模板保留原主题'; for (const id of ['ppt-slide-title', 'ppt-slide-body', 'ppt-layout']) $(`#${id}`).parentElement.hidden = true; $('#ppt-native-slots').hidden = false; $('#ppt-refresh-preview').hidden = false; $('#ppt-refresh-preview').onclick = () => run(() => refreshNativePreview()); $('#ppt-editing-note').textContent = '每个输入对应一个原生文字槽；背景、图形、母版和图片保持原样。文字修改后点击刷新真实预览。'; }
    renderThumbnails(); selectSlide(selected);
  }
  function renderThumbnails() { const list = $('#ppt-slide-list'); list.replaceChildren(); deck.slides.forEach((slide, i) => { const button = document.createElement('button'); button.className = `ppt-slide-item${i === selected ? ' active' : ''}`; button.dataset.slideIndex = i; button.innerHTML = `<span>${i + 1}</span>${deck.native ? `<div class="ppt-native-thumbnail">${nativeImages.has(`${i}:${nativeVersion}`) ? `<img src="${nativeImages.get(`${i}:${nativeVersion}`)}" alt="第${i + 1}页">` : '<span>原版页面</span>'}</div>` : slideSvg(slide, theme(), i, deck.slides.length)}<small>${xml(slide.title || '未命名页面')}</small>`; button.onclick = () => selectSlide(i); list.append(button); }); }
  function selectSlide(i) {
    selected = i; $('#ppt-slide-title').value = deck.slides[i].title; $('#ppt-slide-body').value = deck.slides[i].body.join('\n'); $('#ppt-layout').value = deck.slides[i].layout;
    $('#ppt-up').disabled = i === 0; $('#ppt-down').disabled = i === deck.slides.length - 1; $('#ppt-delete').disabled = deck.slides.length === 1;
    document.querySelectorAll('.ppt-slide-item').forEach((button, n) => button.classList.toggle('active', n === i));
    if (deck.native) {
      const fields = $('#ppt-native-slots'); fields.replaceChildren();
      for (const slot of deck.slides[i].textSlots || []) { const label = document.createElement('label'); label.textContent = `文字 ${slot.index + 1}${slot.emptyPlaceholder ? '（空白占位）' : ''}`; const input = document.createElement('textarea'); input.rows = Math.max(2, Math.min(5, slot.text.split('\n').length + 1)); input.maxLength = 4000; input.value = slot.text; input.dataset.pptSlot = slot.index; input.oninput = () => { slot.text = input.value; dirty = true; nativeVersion++; deck.slides[selected].title = (deck.slides[selected].textSlots.find(s => s.text.trim())?.text || `第 ${selected + 1} 页`).slice(0, 80); renderThumbnails(); status('文字已修改，点击刷新真实预览查看结果'); }; label.append(input); fields.append(label); }
      if (!fields.children.length) fields.textContent = '此页没有可编辑文字；完整图形仍保留。可导出到原生 PPT 应用编辑其他对象。';
    }
    previewSlide();
  }
  function previewSlide() {
    $('#ppt-page-indicator').textContent = `第 ${selected + 1} / ${deck.slides.length} 页`;
    if (!deck.native) { $('#ppt-slide-preview').innerHTML = slideSvg(deck.slides[selected], theme(), selected, deck.slides.length); return; }
    const image = nativeImages.get(`${selected}:${nativeVersion}`);
    if (image) { const img = document.createElement('img'); img.src = image; img.alt = `原版第 ${selected + 1} 页`; $('#ppt-slide-preview').replaceChildren(img); }
    else { $('#ppt-slide-preview').innerHTML = '<div class="ppt-original-preview-note">正在渲染完整原版页面…</div>'; refreshNativePreview().catch(error => { if ($('#ppt-slide-preview') && deck?.native) $('#ppt-slide-preview').textContent = error.message; status(error.message, true); }); }
  }
  async function refreshNativePreview() {
    const page = selected, version = nativeVersion, token = ++previewToken;
    status('正在保留图形渲染原版页面…');
    const result = await invoke('pptNativeRender', { deck, pages: [page], dpi: 96 });
    if (token !== previewToken || version !== nativeVersion || !deck?.native || !$('#ppt-slide-preview')) return;
    const data = result.images?.[0]?.data; if (!data) throw new Error('没有生成原版预览');
    nativeImages.set(`${page}:${version}`, data); renderThumbnails();
    if (selected === page) { const img = document.createElement('img'); img.src = data; img.alt = `原版第 ${page + 1} 页`; $('#ppt-slide-preview').replaceChildren(img); }
    status('原版背景、图形和文字预览已更新');
  }
  async function exportPptx() { if (!deck) throw new Error('请先选择模板或生成演示'); status('正在生成原生 PPTX…'); const r = await invoke('pptExport', { deck }); const saved = await window.rustWord.suite.save(r.data, r.name); if (saved) { dirty = false; status(`已导出 ${r.name}，可在 PowerPoint 中继续编辑`); } return saved; }
  async function rasterize(slide, i) {
    await document.fonts.ready;
    const canvas = document.createElement('canvas'); canvas.width = 1920; canvas.height = 1080; const ctx = canvas.getContext('2d'); ctx.scale(2, 2);
    const cover = slide.layout === 'cover' || slide.layout === 'section', t = theme(), accent = safeColor(t, 'accent1', '#235bab'), fg = cover ? safeColor(t, 'lt1', '#ffffff') : safeColor(t, 'dk1', '#172b42');
    ctx.fillStyle = cover ? accent : safeColor(t, 'lt1', '#ffffff'); ctx.fillRect(0, 0, 960, 540); ctx.fillStyle = cover ? safeColor(t, 'accent2', '#d7e4f4') : accent; if (cover) ctx.fillRect(800, 0, 160, 540); else { ctx.fillRect(0, 0, 12, 540); ctx.fillRect(62, 126, 836, 3); }
    const drawText = (value, x, y, w, size, bold, fill) => { ctx.font = `${bold ? '700 ' : ''}${size}px "PingFang SC","Microsoft YaHei","Noto Sans CJK SC",Arial,sans-serif`; ctx.fillStyle = fill; const rows = []; let row = ''; for (const c of String(value)) { if (c === '\n' || (row && ctx.measureText(row + c).width > w)) { rows.push(row); row = c === '\n' ? '' : c; } else row += c; } rows.push(row); for (const line of rows) { ctx.fillText(line, x, y); y += size * 1.35; } return y; };
    drawText(slide.title, cover ? 64 : 62, cover ? (slide.layout === 'section' ? 225 : 188) : 83, cover ? 690 : 835, cover ? 44 : 32, true, cover ? fg : accent);
    const chars = slide.body.join('').length, size = chars > 1000 ? 13 : chars > 650 ? 16 : chars > 350 ? 19 : 22; let y = cover ? 328 : 184;
    for (const paragraph of slide.body) { if (!cover) { ctx.fillStyle = accent; ctx.beginPath(); ctx.arc(76, y - 7, 3, 0, Math.PI * 2); ctx.fill(); } y = drawText(paragraph, cover ? 68 : 92, y, cover ? 670 : 790, size, false, fg) + 9; }
    drawText(`Rust Word · ${String(i + 1).padStart(2, '0')} / ${String(deck.slides.length).padStart(2, '0')}`, 64, 518, 800, 11, false, fg);
    return { data: canvas.toDataURL('image/png') };
  }
  async function exportImages() { if (!deck) throw new Error('请先选择模板或生成演示'); status('正在渲染逐页图片…'); if (deck.native) { const rendered = await invoke('pptNativeRender', { deck, dpi: 144 }); const result = await invoke('pptExportImages', { images: rendered.images }); await window.rustWord.suite.save(result.data, `${deck.title || '演示'}-原版图片.zip`); status('已导出保留原版背景与对象的逐页 PNG'); return; } const images = []; for (let i = 0; i < deck.slides.length; i++) { images.push(await rasterize(deck.slides[i], i)); status(`已渲染 ${i + 1} / ${deck.slides.length} 页`); } const r = await invoke('pptExportImages', { images }); await window.rustWord.suite.save(r.data, `${deck.title || '演示'}-图片.zip`); status('已导出 1920×1080 PNG 图片合集'); }
  async function openPptx() { if (!await confirmReplace()) return; const file = await window.rustWord.suite.pick('ppt', '#ppt-file-input'); if (!file) return; const result = await invoke('pptImport', { data: file.data }); await setDeck(result, false); status('已从实际 PPTX 页面重新打开可编辑演示'); }
  async function generate(fromDocument = false) {
    if (!await confirmReplace()) return;
    const promise = modal(fromDocument ? '把当前 Word 转成 PPT 大纲' : 'AI 生成 PPT', '<label>演示主题<textarea id="ppt-topic" rows="3" maxlength="600" placeholder="例如：本季度项目进展汇报"></textarea></label><label>页数<input id="ppt-count" type="number" min="3" max="20" value="6"></label><p>生成后先检查和编辑，再导出 PPTX。' + (fromDocument ? '会发送当前文档正文生成演示大纲。' : '会发送你填写的主题。') + '</p>', [['取消', 'cancel'], ['生成大纲', 'generate', true]]);
    $('#ppt-topic').value = fromDocument ? (documentName?.() || '文档汇报') : '';
    if (await promise !== 'generate') return;
    const topic = $('#ppt-topic').value.trim(); if (!topic) throw new Error('请填写演示主题');
    const slideCount = Number($('#ppt-count').value); if (!Number.isInteger(slideCount) || slideCount < 3 || slideCount > 20) throw new Error('页数应在 3–20 之间');
    const req = { topic, slideCount }; if (fromDocument) { const blocks = []; editor.state.doc.descendants((node, pos) => { if (node.isTextblock && node.textContent.trim()) blocks.push({ id: `block-${pos}`, text: node.textContent }); }); if (!blocks.length) throw new Error('当前文档没有正文'); req.blocks = blocks; req.revision = typeof revision === 'function' ? revision() : editor.state.doc.content.size; }
    overlay.classList.remove('hidden'); $('#ppt-main').innerHTML = '<div class="ppt-generating">正在生成演示大纲…<p>你稍后可以逐页修改标题和内容。</p></div>'; status('正在调用已配置的 AI 服务…');
    const result = await invoke('aiPresentation', req); await setDeck(result); status('AI 大纲已生成。检查每页内容，再导出。');
  }
  $('#ppt-library-button').onclick = () => run(openLibrary); $('#ppt-open').onclick = () => run(openPptx);
  actions.pptTemplates = () => run(openLibrary); actions.pptGenerate = () => run(() => generate(false)); actions.pptFromDocument = () => run(() => generate(true)); actions.pptImages = () => run(async () => { if (!deck) return openLibrary(); overlay.classList.remove('hidden'); renderEditor(); await exportImages(); }); actions.pptExport = () => run(exportPptx);
  window.rustWord.ppt = { getDeck: () => deck ? clone(deck) : null, setDeck, isDirty: () => dirty, openLibrary, exportPptx, exportImages, rasterize, slideSvg };
}
