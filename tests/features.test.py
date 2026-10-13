"""Real upgrade acceptance in an owned tab of the persistent codex-chrome.

The Rust server and every generated document use an isolated test directory.
Only synthetic text is sent to the real configured AI. No successful response
is mocked; service failures are recorded as failures. Each CDP request is short,
while AI generation is observed by bounded separate polls.
"""
import base64
import json
import os
from pathlib import Path
import shlex
import time
import traceback
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'test-output'
OUT.mkdir(exist_ok=True)
URL = os.environ.get('WORD_TEST_URL', 'http://127.0.0.1:18766/')
CONFIG = dict(os.environ)
for line in (Path.home() / '.config/codex-chrome-mcp/env').read_text().splitlines():
    line = line.strip().removeprefix('export ')
    if line and not line.startswith('#') and '=' in line:
        key, value = line.split('=', 1)
        tokens = shlex.split(value)
        if tokens:
            CONFIG.setdefault(key, tokens[0])
BASE = CONFIG['CODEX_CHROME_API_URL_LAN']
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))
TAB = None
CHECKS = []
AI_RESULTS = []
OWN_TEMPLATE = None
ERROR = None


def call(name, **arguments):
    if TAB and name != 'chrome_new_tab':
        arguments.setdefault('tab_id', TAB)
    request = urllib.request.Request(BASE + '/api/tools/call',
        data=json.dumps({'name': name, 'arguments': arguments}).encode(),
        headers={'Authorization': 'Bearer ' + CONFIG['CODEX_CHROME_API_TOKEN'], 'Content-Type': 'application/json'})
    result = json.load(OPENER.open(request, timeout=45))
    if not result.get('ok') or result.get('result', {}).get('isError'):
        raise RuntimeError(str(result))
    raw = ''.join(block.get('text', '') for block in result['result']['content'] if block['type'] == 'text')
    return json.loads(raw)


def evaluate(expression):
    result = call('chrome_cdp', method='Runtime.evaluate', params={'expression': expression, 'returnByValue': True, 'awaitPromise': True})
    if result.get('exceptionDetails'):
        raise AssertionError(result['exceptionDetails'])
    return result.get('result', {}).get('value')


def click(selector):
    call('chrome_click', selector=selector)


def type_text(selector, value, clear=True):
    if clear and value == '':
        click(selector)
        call('chrome_keypress', key='Control+A')
        call('chrome_keypress', key='Backspace')
    else:
        call('chrome_type', selector=selector, text=value, clear=clear)


def select_option(selector, value):
    # Operate the native select with actual keyboard events.
    index = evaluate(f"[...document.querySelector({json.dumps(selector)}).options].findIndex(o=>o.value==={json.dumps(value)})")
    assert index >= 0
    click(selector)
    call('chrome_keypress', key='Home')
    for _ in range(index):
        call('chrome_keypress', key='ArrowDown')
    call('chrome_keypress', key='Enter')
    assert evaluate(f"document.querySelector({json.dumps(selector)}).value") == value


def wait_for(expression, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = evaluate(expression)
        if value:
            return value
        time.sleep(.5)
    raise AssertionError(f'Timed out waiting for {expression}')


def check(condition, description):
    if not condition:
        raise AssertionError(description)
    CHECKS.append(description)
    print('PASS ' + description, flush=True)


def screenshot(name):
    image = call('chrome_cdp', method='Page.captureScreenshot', params={'format': 'png'})
    (OUT / name).write_bytes(base64.b64decode(image['data']))


def load(document, name='独立功能验收'):
    evaluate('window.rustWord.loadDocument(' + json.dumps({'document': document, 'name': name}, ensure_ascii=False) + ')')


def select_text(value):
    # Explicit test selection in the real editor; all feature actions use UI.
    return evaluate('(() => {let found;window.rustWord.editor.state.doc.descendants((n,p)=>{if(n.isText&&!found){let i=n.text.indexOf(' + json.dumps(value, ensure_ascii=False) + ');if(i>=0)found={from:p+i,to:p+i+' + str(len(value)) + '};}});if(!found)throw new Error("测试片段不存在");window.rustWord.editor.commands.setTextSelection(found);return found;})()')


def model():
    return evaluate('window.rustWord.document()')


def save_open(path):
    before = model()
    saved = evaluate('window.rustWord.invoke("savePath",{path:' + json.dumps(path) + ',document:window.rustWord.document()})')
    opened = evaluate('window.rustWord.invoke("openPath",{path:' + json.dumps(path) + '})')
    check(opened['document'] == before, 'DOCX 保存重开保留当前完整模型：' + path)
    archive = Path(saved['path'])
    check(archive.is_file() and zipfile.is_zipfile(archive), '实际磁盘生成可读 DOCX ZIP：' + path)
    return saved, opened


def begin_ai(mode, source):
    load({'type': 'doc', 'content': [{'type': 'paragraph', 'content': [{'type': 'text', 'text': source, 'marks': [{'type': 'bold'}]}]}]})
    click('[data-tab="writing"]')
    click('[data-action="ai-' + mode + '"]')
    wait_for("!document.querySelector('#ai-panel').classList.contains('hidden')")
    click('[data-ai-tab="rewrite"]')
    select_text(source)
    click('#ai-generate')
    check(evaluate("document.querySelector('#ai-generate').disabled"), '真实 AI 请求已启动：' + mode)


def finish_ai(mode):
    # A single remote HTTP/CDP call never waits for the AI request itself.
    wait_for("!document.querySelector('#ai-generate').disabled", seconds=240)
    state = evaluate("({source:document.querySelector('#ai-source').value,text:document.querySelector('#ai-result').value,status:document.querySelector('#ai-status').textContent,error:document.querySelector('#ai-status').classList.contains('error'),disabled:document.querySelector('#ai-apply').disabled})")
    AI_RESULTS.append({'mode': mode, **state})
    check(not state['error'] and bool(state['text'].strip()), '真实 Codex 返回修改建议：' + mode)
    return state


try:
    status = call('chrome_status')
    TAB = call('chrome_new_tab', url=URL)['tab']['id']
    (OUT / 'features-tab-id.txt').write_text(TAB)
    wait_for('!!window.rustWord?.features')
    if evaluate("!document.querySelector('#modal').classList.contains('hidden')"):
        # Only dismiss a recovery prompt in this isolated server.
        check(evaluate("document.querySelector('#modal-title').textContent.includes('恢复')"), '隔离测试启动弹窗为草稿恢复')
        click('#modal-actions button:first-child')
    check(evaluate('window.rustWord.features.builtinTemplates.length') == 8, '8 个原创模板已实际加载')
    click('[data-action="templates"]')
    wait_for("!document.querySelector('#template-library').classList.contains('hidden')")
    check(evaluate("document.querySelectorAll('.template-card[data-template-id^=\"builtin-\"]').length") == 8, '模板库显示全部 8 个内置模板')
    screenshot('features-template-library-all.png')
    click('.template-category[data-category="工作"]')
    check(evaluate("document.querySelectorAll('.template-card').length") == 3, '工作分类只显示 3 个对应模板')
    type_text('#template-search', '会议')
    check(evaluate("document.querySelectorAll('.template-card').length") == 1, '模板搜索与分类共同过滤')
    click('.template-card[data-template-id="builtin-meeting"]')
    check(evaluate("document.querySelector('#template-preview-title').textContent") == '会议纪要', '选择会议模板更新预览标题')
    check(evaluate("document.querySelector('#template-preview-paper').textContent.includes('行动事项') && document.querySelectorAll('#template-preview-paper table tr').length === 3"), '模板预览渲染真实正文与行动表格')
    screenshot('features-template-library.png')
    click('#template-use')
    wait_for("document.querySelector('.ProseMirror').textContent.includes('会议纪要') && document.querySelector('#template-library').classList.contains('hidden')")
    check(evaluate("document.querySelectorAll('.ProseMirror table').length") == 1, '会议模板进入实际编辑器含可编辑表格')
    type_text('.ProseMirror > p:nth-of-type(1)', ' 验收正文可编辑', clear=False)
    type_text('.ProseMirror table tr:nth-child(2) td:first-child p', ' 验收表格可编辑', clear=False)
    check(evaluate("window.rustWord.editor.getText().includes('验收正文可编辑') && window.rustWord.editor.getText().includes('验收表格可编辑') && document.querySelectorAll('.ProseMirror table').length===1"), '模板正文和表格经真实键盘输入仍可编辑')
    save_open('features-meeting.docx')

    before_switch = model()
    click('[data-action="templates"]')
    wait_for("!document.querySelector('#template-library').classList.contains('hidden')")
    type_text('#template-search', '')
    click('.template-card[data-template-id="builtin-weekly"]')
    click('#template-use')
    wait_for("document.querySelector('#modal-title').textContent==='保存你的修改？' && !document.querySelector('#modal').classList.contains('hidden')")
    click('#modal-actions button:first-child')
    check(model() == before_switch and evaluate("!document.querySelector('#template-library').classList.contains('hidden')"), '取消模板切换保留未保存正文并返回模板库')
    click('#template-use')
    wait_for("!document.querySelector('#modal').classList.contains('hidden')")
    click('#modal-actions button:nth-child(2)')
    wait_for("document.querySelector('.ProseMirror').textContent.includes('工作周报')")
    check(evaluate("document.querySelector('#template-library').classList.contains('hidden') && !window.rustWord.editor.getText().includes('验收表格可编辑')"), '明确放弃后才切换成工作周报')

    # A dirty guard's save branch uses a real pre-existing test document path.
    saved, opened = save_open('features-guard.docx')
    load(opened['document'], opened['name'])
    type_text('.ProseMirror > p:nth-of-type(1)', ' 保存保护检查', clear=False)
    click('[data-action="templates"]')
    wait_for("!document.querySelector('#template-library').classList.contains('hidden')")
    click('.template-card[data-template-id="builtin-meeting"]')
    click('#template-use')
    wait_for("!document.querySelector('#modal').classList.contains('hidden')")
    click('#modal-actions .primary')
    wait_for("document.querySelector('.ProseMirror').textContent.includes('会议纪要')")
    stored = evaluate("window.rustWord.invoke('openPath',{path:'features-guard.docx'})")
    check('保存保护检查' in json.dumps(stored['document'], ensure_ascii=False), '模板切换选择保存将旧正文真实写入 DOCX')

    # Personal template create, read, update through the backend contract, delete.
    click('[data-action="templates"]')
    wait_for("!document.querySelector('#template-library').classList.contains('hidden')")
    click('#template-save-current')
    wait_for("document.querySelector('#modal-title').textContent==='保存为我的模板'")
    personal_title = '独立验收会议模板 ' + str(int(time.time()))
    type_text('#personal-template-title', personal_title)
    type_text('#personal-template-description', '仅用于功能验收的合成模板')
    click('#modal-actions .primary')
    wait_for("document.querySelector('#template-preview-title').textContent.startsWith('独立验收会议模板')")
    entries = evaluate("window.rustWord.invoke('listTemplates')")['templates']
    personal = next(e for e in entries if e['title'] == personal_title)
    OWN_TEMPLATE = personal['id']
    check(evaluate("document.querySelector('.template-category[data-category=\"我的模板\"]').classList.contains('active') && !document.querySelector('#template-delete').classList.contains('hidden')"), '保存当前文档后在我的模板中可预览与删除')
    updated_title = personal_title + ' 更新'
    evaluate('window.rustWord.invoke("saveTemplate",' + json.dumps({'templateId': OWN_TEMPLATE, 'title': updated_title, 'description': '已更新说明', 'category': '个人', 'document': personal['document']}, ensure_ascii=False) + ')')
    click('#template-close')
    click('[data-action="templates"]')
    wait_for("document.querySelector('#template-preview-title').textContent.endsWith(' 更新')")
    reloaded = evaluate("window.rustWord.invoke('listTemplates')")['templates']
    check(any(e['id'] == OWN_TEMPLATE and e['title'] == updated_title for e in reloaded), '个人模板修改后重新拉取持久化条目')
    templates_path = Path(saved['path']).parent / 'templates.json'
    check(templates_path.is_file() and any(e['id'] == OWN_TEMPLATE and e['title'] == updated_title for e in json.loads(templates_path.read_text())), '个人模板 CRUD 使用实际磁盘 JSON 持久化')
    click('#template-delete')
    wait_for("document.querySelector('#modal-title').textContent==='删除这个模板？'")
    click('#modal-actions button:first-child')
    wait_for("!document.querySelector('#template-library').classList.contains('hidden')")
    check(any(e['id'] == OWN_TEMPLATE for e in evaluate("window.rustWord.invoke('listTemplates')")['templates']), '取消删除保留个人模板')
    click('#template-delete')
    click('#modal-actions .primary')
    wait_for("!document.querySelector('#template-library').classList.contains('hidden')")
    check(not any(e['id'] == OWN_TEMPLATE for e in evaluate("window.rustWord.invoke('listTemplates')")['templates']), '确认删除真实移除个人模板')
    OWN_TEMPLATE = None
    click('#template-close')

    format_doc = {'type': 'doc', 'content': [
        {'type': 'paragraph', 'attrs': {'lineHeight': '2', 'spaceBefore': 6, 'spaceAfter': 18, 'firstLineIndent': 24, 'keepLines': True}, 'content': [{'type': 'text', 'text': '源格式文字', 'marks': [{'type': 'bold'}, {'type': 'textStyle', 'attrs': {'fontSize': '24px'}}]}]},
        {'type': 'paragraph', 'content': [{'type': 'text', 'text': '目标正文文字'}]}]}
    load(format_doc)
    click('[data-tab="home"]')
    select_text('源格式文字')
    click('[data-action="formatPainter"]')
    check(evaluate("document.querySelector('[data-action=\"formatPainter\"]').classList.contains('active')"), '格式刷真实复制源格式并进入待应用状态')
    select_text('目标正文文字')
    target = model()['content'][1]
    marks = target['content'][0].get('marks', [])
    check(any(m['type'] == 'bold' for m in marks) and any(m['type'] == 'textStyle' and m.get('attrs', {}).get('fontSize') == '24px' for m in marks) and target['attrs']['spaceBefore'] == 6 and target['attrs']['spaceAfter'] == 18, '格式刷复制粗体字号和段前后间距到目标选区')
    click('[data-action="undo"]')
    target = model()['content'][1]
    check(not target['content'][0].get('marks') and target['attrs']['spaceBefore'] == 0 and target['attrs']['spaceAfter'] == 8, '一次撤销只撤回格式刷应用')
    select_text('目标正文文字')
    click('[data-action="paragraphSettings"]')
    wait_for("document.querySelector('#modal-title').textContent==='段落设置'")
    for field, value in [('lineHeight', '1.8'), ('spaceBefore', '12'), ('spaceAfter', '16'), ('firstLineIndent', '24')]:
        type_text('#paragraph-' + field, value)
    click('#paragraph-keepNext')
    click('#paragraph-keepLines')
    click('#modal-actions .primary')
    attrs = model()['content'][1]['attrs']
    check(attrs['lineHeight'] == '1.8' and attrs['spaceBefore'] == 12 and attrs['spaceAfter'] == 16 and attrs['firstLineIndent'] == 24 and attrs['keepNext'] and attrs['keepLines'], '段落设置实际按钮修改目标段落模型')
    click('[data-tab="layout"]')
    click('[data-action="pageMargins"]')
    wait_for("document.querySelector('#modal-title').textContent==='页边距'")
    for field, value in [('Top', '24'), ('Bottom', '18'), ('Left', '26'), ('Right', '22')]:
        type_text('#page-margin' + field, value)
    click('#modal-actions .primary')
    margins = model()['attrs']
    check(all(margins['margin' + key] == value for key, value in [('Top', 24), ('Bottom', 18), ('Left', 26), ('Right', 22)]), '页边距对话框四边设置进入模型')
    saved, opened = save_open('features-layout.docx')
    with zipfile.ZipFile(saved['path']) as z:
        xml = z.read('word/document.xml').decode()
    check('w:pgMar' in xml and 'w:spacing' in xml and 'w:keepNext' in xml and 'w:keepLines' in xml, '段落与页边距同时导出为标准 Word XML 属性')

    # UI guards and public configuration do not expose stored secrets.
    click('[data-action="ai"]')
    wait_for("!document.querySelector('#ai-panel').classList.contains('hidden')")
    click('[data-ai-tab="rewrite"]')
    settings = evaluate("window.rustWord.invoke('aiSettings')")
    check(settings['provider'] == 'codex' and 'apiKey' not in settings, '默认 AI 为本机 Codex 且公开配置不包含密钥')
    evaluate('window.rustWord.editor.commands.setTextSelection(1)')
    click('#ai-generate')
    check(evaluate("document.querySelector('#toast').textContent.includes('请先') && !document.querySelector('#ai-generate').disabled"), 'AI 空选择有提示并拒绝发起请求')
    select_option('#ai-mode', 'custom')
    check(evaluate("!document.querySelector('#ai-custom-label').classList.contains('hidden')"), 'AI 模式切换显示自定义要求输入')
    type_text('#ai-instruction', '语气自然，保留所有数字')
    check(evaluate("document.querySelector('#ai-instruction').value") == '语气自然，保留所有数字', '自定义要求可真实输入')
    click('#ai-settings')
    wait_for("document.querySelector('#modal-title').textContent==='AI 服务设置'")
    check(evaluate("document.querySelector('#ai-provider').value==='codex' && document.querySelector('#ai-api-key').value===''"), '服务设置实际展示默认 Codex，密钥输入为空')
    click('#modal-actions button:first-child')

    # Settings preserve a caller-configured local compatible service address.
    click('#ai-settings')
    wait_for("document.querySelector('#modal-title').textContent==='AI 服务设置'")
    select_option('#ai-provider', 'openai')
    type_text('#ai-base-url', 'http://127.0.0.1:18890/v1')
    type_text('#ai-model', 'acceptance-model')
    click('#modal-actions .primary')
    wait_for("document.querySelector('#modal').classList.contains('hidden')")
    click('#ai-settings')
    wait_for("!document.querySelector('#modal').classList.contains('hidden')")
    check(evaluate("document.querySelector('#ai-provider').value==='openai' && document.querySelector('#ai-base-url').value==='http://127.0.0.1:18890/v1'"), '重新打开兼容服务设置保留已保存 localhost 地址')
    select_option('#ai-provider', 'codex')
    click('#modal-actions .primary')
    wait_for("document.querySelector('#modal').classList.contains('hidden')")
    check(evaluate("window.rustWord.invoke('aiSettings')")['provider'] == 'codex', '真实生成前恢复隔离配置为默认 Codex')

    # A full template selection contains a table and must fail before networking.
    meeting_doc = evaluate('window.rustWord.features.builtinTemplates.find(t=>t.id==="builtin-meeting").document')
    load(meeting_doc)
    evaluate('window.rustWord.editor.commands.setTextSelection({from:1,to:window.rustWord.editor.state.doc.content.size-1})')
    click('#ai-generate')
    check(evaluate("document.querySelector('#toast').textContent.includes('跨表格') && !document.querySelector('#ai-generate').disabled"), '跨表格的整份模板 AI 修改被明确拒绝以保护结构')

    formal_source = '咱们明天下午三点开个会，把这周的进度聊一下，请大家提前准备好资料。'
    begin_ai('formal', formal_source)
    formal = finish_ai('formal')
    check(formal['source'] == formal_source and not formal['disabled'] and evaluate('window.rustWord.editor.getText()') == formal_source, '真实 AI 成功后先预览，正文保持原文')
    call('chrome_scroll', x=1100, y=640, delta_y=480)
    check(evaluate("(()=>{const r=document.querySelector('#ai-result').getBoundingClientRect();const b=document.querySelector('.ai-panel-body').getBoundingClientRect();return r.top>=b.top&&r.bottom<=b.bottom;})()"), 'AI 成功预览截图中真实结果全文可见')
    screenshot('features-ai-preview.png')
    click('#ai-apply')
    check(evaluate('window.rustWord.editor.getText()') == formal['text'] and evaluate("document.querySelector('#ai-apply').disabled"), '应用按钮真实替换选区并防止重复应用')
    check(any(mark['type'] == 'bold' for mark in model()['content'][0]['content'][0].get('marks', [])), 'AI 单段替换保留原选区粗体格式')
    screenshot('features-editor-after-ai.png')
    click('[data-action="undo"]')
    check(evaluate('window.rustWord.editor.getText()') == formal_source, '一次撤销恢复 AI 修改前原文')

    stale_source = '这个方案以经完成了，明天可以正式发布。'
    begin_ai('correct', stale_source)
    # Real editing during a request must invalidate the captured source range.
    evaluate("window.rustWord.editor.commands.focus('end')")
    type_text('.ProseMirror', ' 新增的验收内容', clear=False)
    changed = evaluate('window.rustWord.editor.getText()')
    stale = finish_ai('correct')
    check(stale['disabled'] and '正文已经变化' in stale['status'] and changed.endswith('新增的验收内容'), '生成期间真实输入正文后，陈旧建议被禁止应用')
    click('#ai-apply')
    check(evaluate('window.rustWord.editor.getText()') == changed, '点击禁用的陈旧建议应用按钮不会覆盖后来编辑')
    screenshot('features-ai-stale-protection.png')
    evaluate("window.rustWord.invoke('discardRecovery')")
except Exception as error:
    ERROR = {'type': type(error).__name__, 'message': str(error)}
    print('FAIL ' + str(error), flush=True)
    traceback.print_exc()
    if TAB:
        try:
            screenshot('features-failure.png')
        except Exception:
            pass
finally:
    if OWN_TEMPLATE and TAB:
        try:
            evaluate('window.rustWord.invoke("deleteTemplate",{templateId:' + json.dumps(OWN_TEMPLATE) + '})')
        except Exception:
            pass
    if TAB:
        try:
            call('chrome_close_tab')
        except Exception as close_error:
            if not ERROR:
                ERROR = {'type': type(close_error).__name__, 'message': 'Close owned tab: ' + str(close_error)}
    (OUT / 'features-checks.json').write_text(json.dumps({'url': URL, 'tab': TAB, 'passed': len(CHECKS), 'checks': CHECKS, 'ai': AI_RESULTS, 'error': ERROR, 'tabClosed': TAB is not None}, ensure_ascii=False, indent=2) + '\n')
if ERROR:
    raise SystemExit(1)
print(f'{len(CHECKS)} 项真实浏览器功能检查通过；截图与结果位于 {OUT}', flush=True)
