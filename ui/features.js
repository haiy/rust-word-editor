import { installChat } from './chat.js';
import { TextSelection } from '@tiptap/pm/state';
import { closeHistory } from '@tiptap/pm/history';
import { builtinTemplates, previewNodes } from './templates.js';

const $ = selector => document.querySelector(selector);
const $$ = selector => [...document.querySelectorAll(selector)];
const clone = value => JSON.parse(JSON.stringify(value));
const paragraphDefaults = { lineHeight: '1.5', indent: 0, textAlign: 'left', spaceBefore: 0, spaceAfter: 8, firstLineIndent: 0, keepNext: false, keepLines: false, pageBreakBefore: false };
const formatMarks = ['bold', 'italic', 'underline', 'strike', 'textStyle', 'highlight', 'subscript', 'superscript'];

export function installFeatures({ editor, actions, invoke, modal, guard, loadDocument, refresh, toast, revision, documentName, documentIdentity }) {
  let templates = [...builtinTemplates], selectedTemplate = templates[0], category = '全部';
  let brush = null, aiSource = null, aiBusy = false, settings = null, chat = null;
  const library = $('#template-library'), aiPanel = $('#ai-panel');

  function previewTemplate(template) {
    selectedTemplate = template;
    $('#template-preview-title').textContent = template.title;
    $('#template-preview-description').textContent = template.description || '你的个人文档模板';
    const paper = $('#template-preview-paper'); paper.replaceChildren();
    previewNodes(template.document.content, paper);
    $('#template-use').disabled = false;
    $('#template-delete').classList.toggle('hidden', template.id.startsWith('builtin-'));
    $$('.template-card').forEach(card => card.classList.toggle('selected', card.dataset.templateId === template.id));
  }

  function renderTemplates() {
    const query = $('#template-search').value.trim().toLocaleLowerCase();
    const filtered = templates.filter(template => (category === '全部' || (category === '我的模板' ? !template.id.startsWith('builtin-') : template.category === category)) && `${template.title} ${template.category} ${template.description || ''}`.toLocaleLowerCase().includes(query));
    const grid = $('#template-grid'); grid.replaceChildren();
    $('#template-count').textContent = `${filtered.length} 个模板`;
    for (const template of filtered) {
      const card = document.createElement('button'); card.className = 'template-card'; card.dataset.templateId = template.id;
      card.setAttribute('aria-label', `预览 ${template.title}`); card.style.setProperty('--template-accent', template.accent || '#54759b');
      const mini = document.createElement('div'); mini.className = 'template-mini';
      previewNodes(template.document.content.slice(0, 6), mini);
      const title = document.createElement('strong'); title.textContent = template.title;
      const subtitle = document.createElement('span'); subtitle.textContent = template.category;
      card.append(mini, title, subtitle); card.onclick = () => previewTemplate(template); grid.append(card);
    }
    if (!filtered.length) {
      const empty = document.createElement('p'); empty.className = 'templates-empty';
      empty.textContent = category === '我的模板' && !query ? '把一份文档保存为模板，下次就能从这里开始。' : '没有找到匹配的模板，试试其他关键词。'; grid.append(empty);
      $('#template-preview-title').textContent = '选择一个模板'; $('#template-preview-description').textContent = ''; $('#template-preview-paper').replaceChildren(); $('#template-use').disabled = true; $('#template-delete').classList.add('hidden');
    } else previewTemplate(filtered.find(template => template.id === selectedTemplate?.id) || filtered[0]);
  }

  async function openTemplates() {
    const result = await invoke('listTemplates');
    templates = [...builtinTemplates, ...(result.templates || [])];
    renderTemplates(); library.classList.remove('hidden'); $('#template-search').focus();
  }

  async function useTemplate() {
    if (!selectedTemplate) return;
    const template = selectedTemplate; library.classList.add('hidden');
    if (!await guard()) { library.classList.remove('hidden'); return; }
    await invoke('new');
    loadDocument({ document: clone(template.document), name: template.title });
    // The chosen template is a new, unsaved document. Its first edit creates a
    // recovery draft and saving always asks for a new destination.
    editor.view.dispatch(editor.state.tr.setMeta('template-created', true));
    window.rustWord.markTemplateDirty();
    toast(`已从「${template.title}」创建文档，可直接编辑`);
  }

  async function saveTemplate() {
    library.classList.add('hidden');
    const choice = await modal('保存为我的模板', '<label>模板名称<input id="personal-template-title" maxlength="80" placeholder="例如：我的会议纪要"></label><label>说明<input id="personal-template-description" maxlength="160" placeholder="这份模板适合什么场景"></label>', [['取消', 'cancel'], ['保存模板', 'save', true]]);
    if (choice !== 'save') return;
    const title = $('#personal-template-title').value.trim() || documentName().replace(' · 恢复草稿', '');
    await invoke('saveTemplate', { title, category: '个人', description: $('#personal-template-description').value.trim(), document: editor.getJSON() });
    category = '我的模板'; $('#template-search').value = '';
    $$('.template-category').forEach(button => button.classList.toggle('active', button.dataset.category === category));
    await openTemplates(); toast('已保存到本机的我的模板');
  }

  async function deleteTemplate() {
    const template = selectedTemplate;
    if (!template || template.id.startsWith('builtin-')) return;
    library.classList.add('hidden');
    const choice = await modal('删除这个模板？', '<p>删除后将从“我的模板”中移除，不影响已经保存的 Word 文档。</p>', [['取消', 'cancel'], ['删除模板', 'delete', true]]);
    if (choice === 'delete') await invoke('deleteTemplate', { templateId: template.id });
    await openTemplates();
  }

  actions.templates = openTemplates; actions.saveTemplate = saveTemplate;
  $('#template-close').onclick = () => library.classList.add('hidden');
  $('#template-search').oninput = renderTemplates;
  $$('.template-category').forEach(button => button.onclick = () => { category = button.dataset.category; $$('.template-category').forEach(other => other.classList.toggle('active', other === button)); renderTemplates(); });
  $('#template-use').onclick = () => useTemplate().catch(error => toast(error.message));
  $('#template-save-current').onclick = () => saveTemplate().catch(error => toast(error.message));
  $('#template-delete').onclick = () => deleteTemplate().catch(error => toast(error.message));

  function cancelBrush() { brush = null; $('[data-action="formatPainter"]').classList.remove('active'); $('#editor').classList.remove('format-painting'); }
  actions.formatPainter = () => {
    if (brush) { cancelBrush(); return; }
    const selection = editor.state.selection;
    if (!(selection instanceof TextSelection)) { toast('请先选择一段文字或把光标放到正文中'); return; }
    const source = selection.$from.parent;
    const marks = (editor.state.storedMarks || selection.$from.marks()).filter(mark => formatMarks.includes(mark.type.name)).map(mark => mark.toJSON());
    brush = { marks, type: source.type.name, attrs: Object.fromEntries(Object.keys(paragraphDefaults).map(key => [key, source.attrs[key] ?? paragraphDefaults[key]])), level: source.attrs.level };
    $('[data-action="formatPainter"]').classList.add('active'); $('#editor').classList.add('format-painting');
    toast('格式已复制。选中目标文字即可应用，按 Esc 取消');
  };
  editor.on('selectionUpdate', () => {
    if (!brush || editor.state.selection.empty || !(editor.state.selection instanceof TextSelection)) return;
    const format = brush; cancelBrush(); editor.view.dispatch(closeHistory(editor.state.tr));
    let chain = editor.chain().focus();
    for (const type of formatMarks) chain = chain.unsetMark(type);
    for (const mark of format.marks) chain = chain.setMark(mark.type, mark.attrs || {});
    if (format.type === 'heading') chain = chain.setHeading({ level: format.level });
    else if (format.type === 'paragraph') chain = chain.setParagraph();
    chain.updateAttributes('paragraph', format.attrs).updateAttributes('heading', format.attrs).run();
    editor.view.dispatch(closeHistory(editor.state.tr)); toast('已应用格式，可撤销');
  });

  actions.paragraphSettings = async () => {
    const selection = editor.state.selection;
    if (!(selection instanceof TextSelection)) { toast('请把光标放到要设置的段落'); return; }
    const type = selection.$from.parent.type.name === 'heading' ? 'heading' : 'paragraph';
    const attrs = { ...paragraphDefaults, ...editor.getAttributes(type) };
    const fields = [['lineHeight', '行距（倍）', 1, 3, .1], ['spaceBefore', '段前（磅）', 0, 96, 1], ['spaceAfter', '段后（磅）', 0, 96, 1], ['firstLineIndent', '首行缩进（磅）', -36, 72, 1]];
    const body = '<div class="settings-grid">' + fields.map(([key, label, min, max, step]) => `<label>${label}<input id="paragraph-${key}" type="number" min="${min}" max="${max}" step="${step}"></label>`).join('') + '</div><div class="settings-checks"><label class="setting-check"><input id="paragraph-keepNext" type="checkbox">与下一段同页</label><label class="setting-check"><input id="paragraph-keepLines" type="checkbox">段中不分页</label><label class="setting-check"><input id="paragraph-pageBreakBefore" type="checkbox">段前分页</label></div>';
    const promise = modal('段落设置', body, [['取消', 'cancel'], ['应用', 'apply', true]]);
    for (const [key] of fields) $(`#paragraph-${key}`).value = attrs[key];
    for (const key of ['keepNext', 'keepLines', 'pageBreakBefore']) $(`#paragraph-${key}`).checked = attrs[key];
    if (await promise !== 'apply') return;
    const next = {};
    for (const [key, label, min, max] of fields) { const value = Number($(`#paragraph-${key}`).value); if (!Number.isFinite(value) || value < min || value > max) throw new Error(`${label}应在 ${min}–${max} 之间`); next[key] = key === 'lineHeight' ? String(value) : value; }
    for (const key of ['keepNext', 'keepLines', 'pageBreakBefore']) next[key] = $(`#paragraph-${key}`).checked;
    editor.view.dispatch(closeHistory(editor.state.tr));
    editor.chain().focus().setTextSelection({ from: selection.from, to: selection.to }).updateAttributes('paragraph', next).updateAttributes('heading', next).run();
    editor.view.dispatch(closeHistory(editor.state.tr)); refresh();
  };

  actions.pageMargins = async () => {
    const fields = [['marginTop', '上'], ['marginBottom', '下'], ['marginLeft', '左'], ['marginRight', '右']];
    const promise = modal('页边距', '<p>单位：毫米。设置会随 Word 文档保存。</p><div class="settings-grid">' + fields.map(([key, label]) => `<label>${label}<input id="page-${key}" type="number" min="5" max="50" step="1"></label>`).join('') + '</div>', [['取消', 'cancel'], ['应用', 'apply', true]]);
    for (const [key] of fields) $(`#page-${key}`).value = editor.state.doc.attrs[key] ?? 20;
    if (await promise !== 'apply') return;
    let transaction = editor.state.tr;
    for (const [key, label] of fields) { const value = Number($(`#page-${key}`).value); if (!Number.isFinite(value) || value < 5 || value > 50) throw new Error(`${label}页边距应在 5–50 毫米之间`); transaction = transaction.setDocAttribute(key, value); }
    editor.view.dispatch(closeHistory(transaction)); refresh();
  };

  async function aiSettings() {
    const current = await invoke('aiSettings'); settings = current;
    const promise = modal('AI 服务设置', '<label>使用的服务<select id="ai-provider"><option value="codex">本机已登录的 Codex</option><option value="gpu">已有 GPU AI 服务</option><option value="openai">OpenAI 兼容接口</option></select></label><p id="ai-provider-hint"></p><div id="ai-api-fields"><label>接口地址<input id="ai-base-url" placeholder="https://api.example.com/v1"></label><div id="ai-gpu-fields"><label>用户 ID<input id="ai-user-id"></label><label>Codex 配置<input id="ai-codex-profile" value="local"></label></div><div id="ai-openai-fields"><label>模型<input id="ai-model" placeholder="服务支持的模型名称"></label><label>API 密钥<input id="ai-api-key" type="password" autocomplete="new-password"></label><label class="setting-check"><input id="ai-clear-key" type="checkbox">删除已保存的密钥</label></div></div>', [['取消', 'cancel'], ['保存设置', 'save', true]]);
    $('#ai-provider').value = current.provider;
    $('#ai-base-url').value = current.baseUrl || ''; $('#ai-user-id').value = current.userId || ''; $('#ai-model').value = current.model || ''; $('#ai-codex-profile').value = current.codexProfile || 'local';
    $('#ai-api-key').placeholder = current.apiKeySet ? '已保存，留空保持不变' : '仅保存在本机';
    let previousProvider = current.provider;
    const update = () => {
      const provider = $('#ai-provider').value;
      $('#ai-api-fields').classList.toggle('hidden', provider === 'codex'); $('#ai-gpu-fields').classList.toggle('hidden', provider !== 'gpu'); $('#ai-openai-fields').classList.toggle('hidden', provider !== 'openai');
      $('#ai-provider-hint').textContent = { codex: '复用本机账户和当前默认模型，不需要另填密钥。', gpu: '连接你已有的服务，生成建议后由你决定是否应用。', openai: '填写服务提供的地址、模型和密钥。' }[provider];
      if (provider !== previousProvider && provider === 'openai' && (!$('#ai-base-url').value || $('#ai-base-url').value === 'http://127.0.0.1:18090')) $('#ai-base-url').value = 'https://api.openai.com/v1';
      if (provider !== previousProvider && provider === 'gpu' && (!$('#ai-base-url').value || $('#ai-base-url').value === 'https://api.openai.com/v1')) $('#ai-base-url').value = 'http://127.0.0.1:18090';
      previousProvider = provider;
    };
    $('#ai-provider').onchange = update; update();
    if (await promise !== 'save') { $('#ai-api-key').value = ''; return; }
    const next = { provider: $('#ai-provider').value, baseUrl: $('#ai-base-url').value.trim(), userId: $('#ai-user-id').value.trim(), model: $('#ai-model').value.trim(), codexProfile: $('#ai-codex-profile').value.trim(), apiKey: $('#ai-api-key').value, clearApiKey: $('#ai-clear-key').checked };
    $('#ai-api-key').value = ''; settings = await invoke('saveAiSettings', { settings: next });
    updateAiProvider(); toast('AI 服务设置已保存');
  }

  function updateAiProvider() { const label = { codex: '本机 Codex', gpu: '已有 GPU 服务', openai: settings?.model || '兼容接口' }[settings?.provider] || '读取服务配置…'; $('#ai-provider-label').textContent = label; $('#chat-provider-label').textContent = label; }
  async function openAi(mode) {
    $('#find-panel').classList.add('hidden'); aiPanel.classList.remove('hidden'); chat.show(mode ? 'rewrite' : 'chat');
    if (mode) $('#ai-mode').value = mode;
    $('#ai-custom-label').classList.toggle('hidden', $('#ai-mode').value !== 'custom');
    if (!settings) { settings = await invoke('aiSettings'); updateAiProvider(); }
    if (!aiBusy && !$('#ai-result').value) $('#ai-status').textContent = '在正文中选择文字，再生成修改建议。';
  }

  async function runAi() {
    if (aiBusy || chat.isBusy()) { toast('已有一项 AI 请求正在生成，请等待完成'); return; }
    const selection = editor.state.selection;
    if (!(selection instanceof TextSelection) || selection.empty) { toast('请先在正文中选中要修改的文字'); return; }
    const source = editor.state.doc.textBetween(selection.from, selection.to, '\n');
    if (!source.trim()) { toast('选中的内容没有文字'); return; }
    if (!selection.$from.sameParent(selection.$to)) {
      let structuralSelection = false;
      editor.state.doc.nodesBetween(selection.from, selection.to, node => { if (['table', 'image', 'horizontalRule', 'pageBreak'].includes(node.type.name)) structuralSelection = true; });
      if (structuralSelection) { toast('请分段选择正文。跨表格、图片或分页符的修改暂不支持'); return; }
    }
    if ([...source].length > 12000) { toast('每次最多修改 12,000 字，请缩小选择范围'); return; }
    const instruction = $('#ai-instruction').value.trim();
    if ($('#ai-mode').value === 'custom' && !instruction) { toast('请写下你希望怎么修改'); return; }
    aiSource = { from: selection.from, to: selection.to, text: source, revision: revision(), sameBlock: selection.$from.sameParent(selection.$to), marks: selection.$from.marks().filter(mark => mark.type.name !== 'link').map(mark => mark.toJSON()), attrs: clone(selection.$from.parent.attrs) };
    $('#ai-source').value = source; $('#ai-result').value = ''; $('#ai-apply').disabled = true;
    aiBusy = true; $('#ai-generate').disabled = true; $('#ai-generate').textContent = '正在修改…'; $('#ai-status').textContent = '正在生成建议，你可以继续编辑正文。'; $('#ai-status').classList.remove('error');
    try {
      const result = await invoke('aiRewrite', { text: source, mode: $('#ai-mode').value, instruction });
      if (!result.text?.trim()) throw new Error('AI 没有返回可应用的文字，请重新生成');
      $('#ai-result').value = result.text;
      const stale = aiSource.revision !== revision(); $('#ai-apply').disabled = stale;
      $('#ai-status').textContent = stale ? '正文已经变化。为保护修改，请重新选择文字并生成建议。' : '建议已生成。检查或编辑后，再应用到所选文字。';
    } catch (error) { $('#ai-status').textContent = error.message; $('#ai-status').classList.add('error'); }
    finally { aiBusy = false; $('#ai-generate').disabled = false; $('#ai-generate').textContent = '生成修改建议'; }
  }

  function applyAi() {
    const value = $('#ai-result').value;
    if (!aiSource || aiBusy || !value.trim()) return;
    if (aiSource.revision !== revision() || editor.state.doc.textBetween(aiSource.from, aiSource.to, '\n') !== aiSource.text) { $('#ai-apply').disabled = true; $('#ai-status').textContent = '正文已经变化，请重新生成建议。'; return; }
    editor.view.dispatch(closeHistory(editor.state.tr));
    const lines = value.replace(/\r\n?/g, '\n').split('\n');
    const lineContent = line => line ? [{ type: 'text', text: line, marks: aiSource.marks }] : [];
    const content = aiSource.sameBlock ? lines.flatMap((line, index) => [...(index ? [{ type: 'hardBreak' }] : []), ...lineContent(line)]) : lines.map(line => ({ type: 'paragraph', attrs: { ...paragraphDefaults, ...aiSource.attrs }, content: lineContent(line) }));
    const applied = editor.chain().focus().insertContentAt({ from: aiSource.from, to: aiSource.to }, content).run();
    if (!applied) { toast('当前选择无法替换，请选择普通正文再试'); return; }
    editor.view.dispatch(closeHistory(editor.state.tr)); $('#ai-apply').disabled = true; $('#ai-status').textContent = '已应用到正文。可按 ⌘Z 撤销这次修改。'; aiSource = null; toast('AI 修改已应用，可撤销');
  }

  chat = installChat({editor,invoke,modal,toast,revision,documentIdentity,openSettings:aiSettings,isRewriteBusy:()=>aiBusy});
  actions.ai = () => openAi(); actions.aiSettings = aiSettings;
  for (const mode of ['polish', 'correct', 'shorten', 'expand', 'formal']) actions[`ai-${mode}`] = () => openAi(mode);
  $('#ai-close').onclick = () => aiPanel.classList.add('hidden');
  $('#ai-mode').onchange = () => $('#ai-custom-label').classList.toggle('hidden', $('#ai-mode').value !== 'custom');
  $('#ai-generate').onclick = () => runAi().catch(error => toast(error.message));
  $('#ai-apply').onclick = applyAi;
  $('#ai-discard').onclick = () => { if (aiBusy) return; aiSource = null; $('#ai-result').value = ''; $('#ai-apply').disabled = true; $('#ai-status').textContent = '已放弃建议，正文保持原样。'; };
  $('#ai-settings').onclick = () => aiSettings().catch(error => toast(error.message));
  editor.on('update', () => { if (aiSource && aiSource.revision !== revision()) { $('#ai-apply').disabled = true; if ($('#ai-result').value) $('#ai-status').textContent = '正文已经变化，请重新生成建议。'; } });
  document.addEventListener('keydown', event => { if (event.key === 'Escape') { cancelBrush(); library.classList.add('hidden'); if ($('#modal').classList.contains('hidden')) aiPanel.classList.add('hidden'); } });

  window.rustWord.features = { builtinTemplates, openTemplates, openAi, runAi, applyAi };
}
