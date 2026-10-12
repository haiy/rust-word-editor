"""Document boundaries reset undo and search in a real owned Chrome tab.

Two synthetic documents exercise the production loadDocument path, followed by
an original built-in template and real table commands. No AI request is made.
"""
import base64
import json
import os
from pathlib import Path
import shlex
import time
import traceback
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'test-output'
OUT.mkdir(exist_ok=True)
URL = os.environ.get('WORD_TEST_URL', 'http://127.0.0.1:18766/')
CONFIG = dict(os.environ)
for line in (Path.home() / '.config/codex-chrome-mcp/env').read_text().splitlines():
    line = line.strip().removeprefix('export ')
    if line and not line.startswith('#') and '=' in line:
        key, value = line.split('=', 1)
        words = shlex.split(value)
        if words:
            CONFIG.setdefault(key, words[0])
BASE = CONFIG['CODEX_CHROME_API_URL_LAN']
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))
TAB = None
CHECKS = []
ERROR = None
CLOSED = False


def call(name, **arguments):
    if TAB and name != 'chrome_new_tab':
        arguments.setdefault('tab_id', TAB)
    request = urllib.request.Request(BASE + '/api/tools/call',
        data=json.dumps({'name': name, 'arguments': arguments}).encode(),
        headers={'Authorization': 'Bearer ' + CONFIG['CODEX_CHROME_API_TOKEN'], 'Content-Type': 'application/json'})
    result = json.load(OPENER.open(request, timeout=45))
    if not result.get('ok') or result.get('result', {}).get('isError'):
        raise RuntimeError(str(result))
    text = ''.join(block.get('text', '') for block in result['result']['content'] if block['type'] == 'text')
    return json.loads(text)


def evaluate(expression):
    result = call('chrome_cdp', method='Runtime.evaluate', params={'expression': expression, 'returnByValue': True, 'awaitPromise': True})
    if result.get('exceptionDetails'):
        raise AssertionError(result['exceptionDetails'])
    return result.get('result', {}).get('value')


def click(selector):
    call('chrome_click', selector=selector)


def type_text(selector, value, clear=True):
    call('chrome_type', selector=selector, text=value, clear=clear)


def wait_for(expression, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if evaluate(expression):
            return
        time.sleep(.25)
    raise AssertionError('Timeout: ' + expression)


def check(condition, message):
    if not condition:
        raise AssertionError(message)
    CHECKS.append(message)
    print('PASS ' + message, flush=True)


def load(document, name):
    evaluate('window.rustWord.loadDocument(' + json.dumps({'document': document, 'name': name}, ensure_ascii=False) + ')')


def model():
    return evaluate('window.rustWord.document()')


def history():
    return evaluate("({undo:window.rustWord.editor.can().undo(),redo:window.rustWord.editor.can().redo(),undoDisabled:document.querySelector('[data-action=\"undo\"]').disabled,redoDisabled:document.querySelector('[data-action=\"redo\"]').disabled})")


def screenshot(name):
    result = call('chrome_cdp', method='Page.captureScreenshot', params={'format': 'png'})
    (OUT / name).write_bytes(base64.b64decode(result['data']))


DOC_A = {'type': 'doc', 'content': [
    {'type': 'paragraph', 'content': [{'type': 'text', 'text': 'A文档包含查找目标，查找目标。'}]},
    {'type': 'paragraph'}]}
DOC_B = {'type': 'doc', 'content': [
    {'type': 'paragraph', 'content': [{'type': 'text', 'text': 'B文档正文：新的工作计划。'}]},
    {'type': 'paragraph'}]}

try:
    call('chrome_status')
    TAB = call('chrome_new_tab', url=URL)['tab']['id']
    (OUT / 'document-switch-tab-id.txt').write_text(TAB)
    wait_for('!!window.rustWord?.features')
    if evaluate("!document.querySelector('#modal').classList.contains('hidden')"):
        if evaluate("document.querySelector('#modal-title').textContent.includes('恢复')"):
            click('#modal-actions button:first-child')
        else:
            raise AssertionError('Unexpected isolated startup dialog')
    load(DOC_A, '合成文档A')
    type_text('.ProseMirror > p:first-child', ' A新编辑片段', clear=False)
    check('A新编辑片段' in evaluate('window.rustWord.editor.getText()') and history()['undo'], 'A 文档真实输入产生自身撤销历史')
    click('[data-action="find"]')
    type_text('#search', '查找目标')
    type_text('#replace', '不应写入B的替换文本')
    check(evaluate("document.querySelectorAll('.find-match').length") == 2, 'A 文档真实查找建立两处匹配')

    load(DOC_B, '合成文档B')
    baseline_b = model()
    state = history()
    check(not state['undo'] and not state['redo'] and state['undoDisabled'] and state['redoDisabled'], '加载 B 后撤销重做及对应按钮均不可用')
    check(evaluate("document.querySelector('#search').value==='' && document.querySelectorAll('.find-match').length===0 && document.querySelector('#match-count').textContent==='0 / 0 处匹配'"), '切换 B 清空查询与所有匹配高亮、计数')
    click('[data-action="undo"]')
    click('[data-action="redo"]')
    check(model() == baseline_b and not 'A文档' in evaluate('window.rustWord.editor.getText()'), '点击不可用撤销重做不能恢复先前 A 文档')
    click('[data-action="replaceAll"]')
    check(model() == baseline_b and not '不应写入B' in evaluate('window.rustWord.editor.getText()'), '清空匹配后全部替换不使用 A 的旧位置')
    click('#find-close')

    type_text('.ProseMirror > p:first-child', ' B新增编辑', clear=False)
    edited_b = model()
    check('B新增编辑' in evaluate('window.rustWord.editor.getText()') and history()['undo'], 'B 新编辑建立独立有效的撤销历史')
    click('[data-action="undo"]')
    check(model() == baseline_b and not 'A文档' in evaluate('window.rustWord.editor.getText()'), 'B 一次撤销回到原 B 正文，不回到 A')
    click('[data-action="redo"]')
    check(model() == edited_b, 'B 自身编辑重做恢复新增内容')
    click('[data-action="undo"]')
    check(not history()['undo'], 'B 撤到自身初始边界后无法越过文档边界')

    # Table plugin state must remain usable after the EditorState reset.
    evaluate("window.rustWord.editor.commands.focus('end')")
    click('[data-tab="insert"]')
    click('[data-action="tableDialog"]')
    wait_for("document.querySelector('#modal-title').textContent==='插入表格'")
    type_text('#table-rows', '3')
    type_text('#table-cols', '2')
    click('#modal-actions .primary')
    check(evaluate("document.querySelectorAll('.ProseMirror table tr').length") == 3, '插件重置后 B 文档实际插入表格仍有效')
    click('[data-action="addRowAfter"]')
    check(evaluate("document.querySelectorAll('.ProseMirror table tr').length") == 4, '插件重置后表格增行按钮仍有效')
    click('[data-action="deleteRow"]')
    check(evaluate("document.querySelectorAll('.ProseMirror table tr').length") == 3, '插件重置后表格删行按钮仍有效')
    type_text('.ProseMirror table tr:nth-child(2) td:first-child p', 'B表格实际输入', clear=False)
    check('B表格实际输入' in evaluate('window.rustWord.editor.getText()'), '插件重置后表格单元格仍可真实编辑')
    before_template = model()
    click('.topbar [data-action="templates"]')
    wait_for("!document.querySelector('#template-library').classList.contains('hidden')")
    click('.template-card[data-template-id="builtin-meeting"]')
    click('#template-use')
    wait_for("!document.querySelector('#modal').classList.contains('hidden')")
    check(evaluate("document.querySelector('#modal-title').textContent") == '保存你的修改？', '从已编辑 B 切换模板先显示保存保护')
    click('#modal-actions button:first-child')
    check(model() == before_template, '取消模板切换保持 B 内容与表格')
    click('#template-use')
    wait_for("!document.querySelector('#modal').classList.contains('hidden')")
    click('#modal-actions button:nth-child(2)')
    wait_for("document.querySelector('.ProseMirror').textContent.includes('会议纪要') && document.querySelector('#template-library').classList.contains('hidden')")
    template_model = model()
    state = history()
    check(evaluate("window.rustWord.editor.getText().includes('行动事项') && !window.rustWord.editor.getText().includes('B表格实际输入')"), '确认放弃后新内容真实来自会议模板')
    check(not state['undo'] and not state['redo'] and state['undoDisabled'] and state['redoDisabled'], '新模板独立撤销历史，无法回到旧 B 文档')
    click('[data-action="undo"]')
    check(model() == template_model, '模板创建后点击撤销不会恢复先前 B 内容')
    check(evaluate("document.querySelector('#document-name').textContent.startsWith('● ')"), '模板创建后标为尚未保存的新文档')
    click('.topbar [data-action="new"]')
    wait_for("!document.querySelector('#modal').classList.contains('hidden')")
    check(evaluate("document.querySelector('#modal-title').textContent") == '保存你的修改？', '刚创建且未编辑的模板再次新建仍有保存保护')
    click('#modal-actions button:first-child')
    check(model() == template_model, '取消新建保留新模板正文')
    screenshot('document-switch-template.png')

    # No AI call is initiated. Existing real stale-result behavior is covered by
    # features.test.py; a clean new tab must not enable an ungenerated suggestion.
    click('.quick-actions [data-action="ai"]')
    wait_for("!document.querySelector('#ai-panel').classList.contains('hidden')")
    load(DOC_B, '重新加载合成文档B')
    check(evaluate("document.querySelector('#ai-result').value==='' && document.querySelector('#ai-apply').disabled"), '无已生成建议时切换文档不会启用 AI 采纳')
    check(not history()['undo'] and not history()['redo'], '从模板重新加载 B 再次清空撤销边界')
    evaluate("window.rustWord.invoke('discardRecovery')")
except Exception as error:
    ERROR = {'type': type(error).__name__, 'message': str(error)}
    print('FAIL ' + str(error), flush=True)
    traceback.print_exc()
    if TAB:
        try:
            screenshot('document-switch-failure.png')
        except Exception:
            pass
finally:
    if TAB:
        try:
            call('chrome_close_tab')
            CLOSED = True
        except Exception as error:
            if ERROR is None:
                ERROR = {'type': type(error).__name__, 'message': 'Close owned tab: ' + str(error)}
    (OUT / 'document-switch-checks.json').write_text(json.dumps({'url': URL, 'tab': TAB, 'passed': len(CHECKS), 'checks': CHECKS, 'error': ERROR, 'tabClosed': CLOSED, 'aiRequests': 0}, ensure_ascii=False, indent=2) + '\n')
if ERROR:
    raise SystemExit(1)
print(f'{len(CHECKS)} 项文档切换隔离检查通过，未发起 AI 请求', flush=True)
