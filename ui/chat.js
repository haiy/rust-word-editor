import { DOMSerializer } from '@tiptap/pm/model';
import { TextSelection } from '@tiptap/pm/state';
import { closeHistory } from '@tiptap/pm/history';

const $ = selector => document.querySelector(selector);
const clone = value => JSON.parse(JSON.stringify(value));

// Parse a small Markdown subset into editor nodes, never model-supplied HTML.
export function answerBlocks(text) {
  const inline = line => {
    const result = [];
    let offset = 0;
    for (const match of line.matchAll(/\*\*([^*]+)\*\*|`([^`]+)`/g)) {
      if (match.index > offset) result.push({type:'text',text:line.slice(offset,match.index)});
      result.push({type:'text',text:match[1] || match[2],marks:[{type:match[1] ? 'bold' : 'code'}]});
      offset = match.index + match[0].length;
    }
    if (offset < line.length) result.push({type:'text',text:line.slice(offset)});
    return result;
  };
  const blocks = [];
  let paragraph = [], code = null;
  const flush = () => {
    if (paragraph.length) blocks.push({type:'paragraph',content:inline(paragraph.join('\n'))});
    paragraph = [];
  };
  for (const line of text.replace(/\r\n?/g,'\n').split('\n')) {
    if (/^\s*```/.test(line)) {
      flush();
      if (code === null) code = [];
      else { blocks.push({type:'codeBlock',content:code.length ? [{type:'text',text:code.join('\n') || ' '}] : []}); code = null; }
      continue;
    }
    if (code !== null) { code.push(line); continue; }
    if (!line.trim()) { flush(); continue; }
    const heading = line.match(/^(#{1,6})\s+(.+)$/);
    const bullet = line.match(/^\s*[-*+]\s+(.+)$/);
    const numbered = line.match(/^\s*(\d+)[.)]\s+(.+)$/);
    if (heading) { flush(); blocks.push({type:'heading',attrs:{level:heading[1].length},content:inline(heading[2])}); }
    else if (bullet || numbered) {
      flush();
      const type = bullet ? 'bulletList' : 'orderedList';
      let list = blocks[blocks.length - 1];
      if (list?.type !== type) { list = {type,content:[]}; if (numbered) list.attrs = {start:Number(numbered[1])}; blocks.push(list); }
      list.content.push({type:'listItem',content:[{type:'paragraph',content:inline(bullet ? bullet[1] : numbered[2])}]});
    } else paragraph.push(line);
  }
  if (code !== null) blocks.push({type:'codeBlock',content:code.length ? [{type:'text',text:code.join('\n') || ' '}] : []});
  flush();
  return blocks.length ? blocks : [{type:'paragraph'}];
}

export function installChat({editor,invoke,modal,toast,revision,documentIdentity,openSettings,selectProvider,isRewriteBusy}) {
  let turns = [], context = null, busy = false, changingProvider = false, activeTab = 'chat';
  const input = $('#chat-input'), messages = $('#chat-messages');
  function status(text, error = false) {
    $('#chat-status').textContent = text;
    $('#chat-status').classList.toggle('error',error);
  }
  function controls() {
    $('#chat-send').disabled = busy || changingProvider || !input.value.trim();
    $('#chat-send').textContent = busy ? '正在回复…' : '发送';
    $('#chat-new').disabled = busy || changingProvider;
    $('#chat-provider-select').disabled = busy || changingProvider;
    $('#chat-settings').disabled = busy || changingProvider;
    input.style.height = 'auto'; input.style.height = Math.min(150,Math.max(66,input.scrollHeight))+'px';
  }
  function renderContext() {
    $('#chat-context').classList.toggle('hidden',!context);
    $('#chat-context-label').textContent = context ? `${context.kind === 'selection' ? '引用选文' : '引用全文'} · ${[...context.text].length.toLocaleString()} 字` : '';
    $('#chat-context-preview').textContent = context?.text || '';
  }
  function canReplace(turn) {
    const source = turn.source;
    return !busy && !!source && !turn.applied && turn.documentId === documentIdentity()
      && source.revision === revision() && source.to <= editor.state.doc.content.size
      && editor.state.doc.textBetween(source.from,source.to,'\n') === source.text;
  }
  function updateApplyButtons() {
    for (const button of messages.querySelectorAll('[data-replace-turn]')) button.disabled = !canReplace(turns[Number(button.dataset.replaceTurn)]);
  }
  function button(label, handler) {
    const el = document.createElement('button'); el.textContent = label;
    el.onclick = () => Promise.resolve().then(handler).catch(error => status(error.message,true));
    return el;
  }
  async function insert(turn) {
    if (!turn.answer) return;
    if (turn.documentId !== documentIdentity()) {
      if (await modal('插入当前文档？','<p>这条回答来自另一份文档的对话。确认后将追加到当前文档末尾。</p>',[['取消','cancel'],['插入文末','insert',true]]) !== 'insert') return;
    }
    const blocks = answerBlocks(turn.answer);
    editor.schema.nodeFromJSON({type:'doc',content:blocks}).check();
    editor.view.dispatch(closeHistory(editor.state.tr));
    if (!editor.chain().focus().insertContentAt(editor.state.doc.content.size,blocks).run()) throw new Error('无法插入正文，请重试');
    editor.view.dispatch(closeHistory(editor.state.tr));
    toast('已插入文末，可按 ⌘Z 撤销');
  }
  function replace(turn) {
    if (!canReplace(turn)) { status('正文已经变化，请重新引用选文后再生成回答。',true); return; }
    const source = turn.source;
    const lines = turn.answer.replace(/\r\n?/g,'\n').split('\n');
    const content = source.sameBlock
      ? lines.flatMap((text,index) => [...(index ? [{type:'hardBreak'}] : []),...(text ? [{type:'text',text,marks:source.marks}] : [])])
      : lines.map(text => ({type:'paragraph',attrs:source.attrs,content:text ? [{type:'text',text,marks:source.marks}] : []}));
    editor.view.dispatch(closeHistory(editor.state.tr));
    if (!editor.chain().focus().insertContentAt({from:source.from,to:source.to},content).run()) throw new Error('当前选择无法替换，请选择普通正文再试');
    editor.view.dispatch(closeHistory(editor.state.tr)); turn.applied = true; updateApplyButtons();
    toast('已替换引用选文，可按 ⌘Z 撤销');
  }
  async function copy(text) {
    try { await navigator.clipboard.writeText(text); }
    catch {
      const el = document.createElement('textarea'); el.value = text; el.style.cssText = 'position:fixed;left:-9999px'; document.body.append(el); el.select();
      const copied = document.execCommand('copy'); el.remove();
      if (!copied) throw new Error('复制失败，请选中回答后手动复制');
    }
    toast('已复制回答');
  }
  function render(scroll = false) {
    const scrollbox = document.querySelector('.chat-scroll');
    const oldScroll = scrollbox.scrollTop;
    messages.replaceChildren();
    $('#chat-empty').classList.toggle('hidden',turns.length > 0);
    turns.forEach((turn,index) => {
      const user = document.createElement('article'); user.className = 'chat-message chat-user';
      const label = document.createElement('span'); label.className = 'chat-speaker'; label.textContent = '你';
      const body = document.createElement('div'); body.className = 'chat-user-text'; body.textContent = turn.question;
      user.append(label,body);
      if (turn.context) {
        const details = document.createElement('details'), summary = document.createElement('summary'), excerpt = document.createElement('pre');
        summary.textContent = turn.context.kind === 'selection' ? '引用的选文' : '引用的全文'; excerpt.textContent = turn.context.text;
        details.append(summary,excerpt); user.append(details);
      }
      messages.append(user);
      const reply = document.createElement('article'); reply.className = 'chat-message chat-assistant';
      const heading = document.createElement('span'); heading.className = 'chat-speaker'; heading.textContent = turn.provider ? `AI 助手 · ${providerName(turn.provider)}` : 'AI 助手'; if (turn.model) heading.append(document.createTextNode(' · '+turn.model)); reply.append(heading);
      if (turn.answer) {
        const content = document.createElement('div'); content.className = 'chat-answer';
        const doc = editor.schema.nodeFromJSON({type:'doc',content:answerBlocks(turn.answer)});
        content.append(DOMSerializer.fromSchema(editor.schema).serializeFragment(doc.content)); reply.append(content);
        const actions = document.createElement('div'); actions.className = 'chat-message-actions';
        actions.append(button('复制',() => copy(turn.answer)),button('插入文末',() => insert(turn)));
        if (turn.source) { const apply = button('替换引用选文',() => replace(turn)); apply.dataset.replaceTurn = String(index); apply.disabled = !canReplace(turn); actions.append(apply); }
        reply.append(actions);
      } else if (turn.error) {
        const error = document.createElement('p'); error.className = 'chat-error'; error.textContent = turn.error; reply.append(error);
        if (index === turns.length - 1) { const retry = button('重试',() => send(turn)); retry.disabled = busy; reply.append(retry); }
      } else { const waiting = document.createElement('p'); waiting.className = 'chat-waiting'; waiting.textContent = '正在思考…'; reply.append(waiting); }
      messages.append(reply);
    });
    if (scroll) scrollbox.scrollTop = scrollbox.scrollHeight;
    else scrollbox.scrollTop = oldScroll;
  }
  function attach(kind) {
    if (busy) return;
    const selection = editor.state.selection;
    let text, source = null;
    if (kind === 'selection') {
      if (!(selection instanceof TextSelection) || selection.empty) { toast('先在正文中选择一段文字，再引用到对话'); return; }
      text = editor.state.doc.textBetween(selection.from,selection.to,'\n');
      let structural = false;
      editor.state.doc.nodesBetween(selection.from,selection.to,node => { if (['table','image','horizontalRule','pageBreak'].includes(node.type.name)) structural = true; });
      if (!structural) source = {from:selection.from,to:selection.to,text,revision:revision(),sameBlock:selection.$from.sameParent(selection.$to),marks:selection.$from.marks().filter(mark => mark.type.name !== 'link').map(mark => mark.toJSON()),attrs:clone(selection.$from.parent.attrs)};
    } else text = editor.getText({blockSeparator:'\n'});
    if (!text.trim()) { toast('当前引用没有文字'); return; }
    if ([...text].length > 120000) { toast('引用正文超过 120000 字，请改用选文'); return; }
    context = {kind,text,source,documentId:documentIdentity()}; renderContext(); input.focus({preventScroll:true});
  }
  async function send(retry = null) {
    if (busy || changingProvider || isRewriteBusy()) { toast('已有一项 AI 请求正在生成，请等待完成'); return; }
    const question = retry ? retry.question : input.value.trim();
    if (!question) return;
    if ([...question].length > 8000) { status('提问最多 8000 字，请缩短后再发送',true); return; }
    const turn = retry || {question,context:context ? {kind:context.kind,text:context.text} : null,source:context?.source ? clone(context.source) : null,documentId:documentIdentity(),answer:null,error:null};
    const history = [];
    for (const prior of turns) {
      if (prior === retry) break;
      if (!prior.answer) continue;
      history.push({role:'user',content:prior.question,...(prior.context ? {context:prior.context} : {})},{role:'assistant',content:prior.answer});
    }
    history.push({role:'user',content:question,...(turn.context ? {context:turn.context} : {})});
    if (history.length > 39) { status('对话最多支持 20 轮，请新建对话继续',true); return; }
    if (!retry) { turns.push(turn); input.value = ''; context = null; renderContext(); }
    turn.provider = $('#chat-provider-select').value; turn.error = null; busy = true; controls(); render(true); status('正在回复，你可以继续编辑文档。');
    try {
      const result = await invoke('aiChat',{messages:history,provider:turn.provider});
      if (!result.text?.trim()) throw new Error('AI 没有返回文字，请重试');
      turn.provider = result.provider || turn.provider; turn.model = result.model || ''; turn.answer = result.text; status('可以继续追问，或将回答插入文档。');
    } catch (error) { turn.error = error.message; status('回复失败，可以重试或修改问题后发送。',true); }
    finally { busy = false; controls(); render(true); }
  }
  function show(tab = 'chat') {
    activeTab = tab;
    $('#ai-chat').classList.toggle('hidden',tab !== 'chat');
    $('#ai-rewrite').classList.toggle('hidden',tab !== 'rewrite');
    for (const el of document.querySelectorAll('[data-ai-tab]')) {
      const selected = el.dataset.aiTab === tab; el.classList.toggle('active',selected); el.setAttribute('aria-selected',String(selected));
    }
    if (tab === 'chat') input.focus({preventScroll:true});
  }
  $('#chat-new').onclick = async () => {
    if (busy) return;
    if ((turns.length || input.value.trim()) && await modal('开始新对话？','<p>当前窗口的聊天记录将清空，文档内容保持不变。</p>',[['取消','cancel'],['新对话','new',true]]) !== 'new') return;
    turns = []; context = null; input.value = ''; renderContext(); render(); controls(); status(''); input.focus({preventScroll:true});
  };
  $('#chat-send').onclick = () => send();
  input.oninput = controls;
  input.onkeydown = event => {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing && event.keyCode !== 229) { event.preventDefault(); void send(); }
  };
  $('#chat-reference-selection').onclick = () => attach('selection');
  $('#chat-reference-document').onclick = () => attach('document');
  $('#chat-clear-context').onclick = () => { context = null; renderContext(); };
  function providerName(provider) { return {codex:'Codex',workbuddy:'WorkBuddy',gpu:'已有 GPU 服务',openai:'兼容接口'}[provider] || provider; }
  $('#chat-provider-select').onchange = async event => {
    const provider = event.target.value;
    if (busy || changingProvider) return;
    changingProvider = true; controls();
    try {
      await selectProvider(provider);
      status(`已选择 ${providerName(provider)}，下一条消息将携带当前对话历史。`);
    } catch (error) {
      const saved = await invoke('aiSettings').catch(()=>null);
      if (saved) event.target.value = saved.provider;
      status(error.message,true);
    } finally { changingProvider = false; controls(); }
  };
  $('#chat-settings').onclick = () => openSettings().catch(error => toast(error.message));
  for (const el of document.querySelectorAll('[data-chat-prompt]')) el.onclick = () => { input.value = el.dataset.chatPrompt; controls(); input.focus({preventScroll:true}); };
  for (const el of document.querySelectorAll('[data-ai-tab]')) el.onclick = () => show(el.dataset.aiTab);
  editor.on('update',updateApplyButtons);
  const api = {
    show, isBusy:() => busy || changingProvider,
    documentChanged() { context = null; renderContext(); updateApplyButtons(); },
    snapshot:() => ({turns:clone(turns),context:context ? clone(context) : null,draft:input.value,tab:activeTab,documentId:documentIdentity()}),
    restore(snapshot) {
      if (!snapshot || !Array.isArray(snapshot.turns)) return;
      turns = clone(snapshot.turns).slice(-24);
      for (const turn of turns) turn.documentId = turn.documentId === snapshot.documentId ? documentIdentity() : -1;
      context = snapshot.context ? clone(snapshot.context) : null;
      if (context) context.documentId = documentIdentity();
      input.value = snapshot.draft || ''; show(snapshot.tab); renderContext(); render(true); controls();
    },
  };
  window.rustWord.chat = api;
  controls(); render(); show();
  return api;
}
