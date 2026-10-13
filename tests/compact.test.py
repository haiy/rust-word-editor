"""Compact toolbar acceptance in an owned persistent codex-chrome tab.

Measures the actual rendered command area and every core command at normal
and minimum desktop viewport sizes. No AI request or native document is used.
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
        tokens = shlex.split(value)
        if tokens:
            CONFIG.setdefault(key, tokens[0])
BASE = CONFIG['CODEX_CHROME_API_URL_LAN']
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))
TAB = None
CHECKS = []
MATRIX = []
ISSUES = []
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


def wait_for(expression, timeout=15):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        if evaluate(expression):
            return
        time.sleep(.25)
    raise AssertionError('Timeout: ' + expression)


def check(condition, description, details=None):
    if condition:
        CHECKS.append(description)
        print('PASS ' + description, flush=True)
    else:
        issue = {'description': description, 'details': details}
        ISSUES.append(issue)
        print('FAIL ' + json.dumps(issue, ensure_ascii=False), flush=True)


def screenshot(name):
    data = call('chrome_cdp', method='Page.captureScreenshot', params={'format': 'png'})
    (OUT / name).write_bytes(base64.b64decode(data['data']))


MEASURE = r"""(selectors => {
  const rect = el => {const r=el.getBoundingClientRect();return {left:r.left,top:r.top,right:r.right,bottom:r.bottom,width:r.width,height:r.height};};
  const visible = el => {
    if(!el) return {visible:false,reason:'missing'};
    const r=el.getBoundingClientRect(), cs=getComputedStyle(el);
    if(!r.width||!r.height||cs.visibility==='hidden'||cs.display==='none') return {visible:false,reason:'hidden',rect:rect(el)};
    if(r.left<-.5||r.right>innerWidth+.5||r.top<-.5||r.bottom>innerHeight+.5) return {visible:false,reason:'viewport-clipping',rect:rect(el)};
    for(let p=el.parentElement;p;p=p.parentElement){
      const s=getComputedStyle(p),b=p.getBoundingClientRect();
      if(['hidden','auto','scroll','clip'].includes(s.overflowX)&&(r.left<b.left-.5||r.right>b.right+.5)) return {visible:false,reason:'ancestor-x-clipping',ancestor:p.className,rect:rect(el)};
      if(['hidden','auto','scroll','clip'].includes(s.overflowY)&&(r.top<b.top-.5||r.bottom>b.bottom+.5)) return {visible:false,reason:'ancestor-y-clipping',ancestor:p.className,rect:rect(el)};
    }
    return {visible:true,rect:rect(el),fontSize:cs.fontSize};
  };
  const ribbon=document.querySelector('.ribbon:not(.hidden)');
  const parts=['.topbar','.tabbar','.ribbon:not(.hidden)'].map(selector=>({selector,...rect(document.querySelector(selector))}));
  return {viewport:{width:innerWidth,height:innerHeight},parts,totalHeight:parts.reduce((sum,p)=>sum+p.height,0),workspaceTop:document.querySelector('.workspace').getBoundingClientRect().top,ribbonScrollWidth:ribbon.scrollWidth,ribbonClientWidth:ribbon.clientWidth,commands:selectors.map(selector=>({selector,...visible(document.querySelector(selector))}))};
})"""
CORE = {
    'home': ['#font-family', '#font-size', '[data-action="formatPainter"]', '[data-action="bold"]', '[data-action="paragraphSettings"]', '[data-action="h1"]', '[data-action="h2"]', '[data-action="h3"]'],
    'insert': ['[data-action="tableDialog"]', '[data-action="image"]', '[data-action="link"]', '[data-action="pageBreak"]', '[data-action="superscript"]', '[data-action="subscript"]'],
    'layout': ['[data-action="portrait"]', '[data-action="landscape"]', '[data-action="pageMargins"]', '[data-action="paragraphSettings"]', '[data-action="print"]'],
    'writing': ['[data-action="templates"]', '[data-action="saveTemplate"]', '[data-action="ai-polish"]', '[data-action="ai-correct"]', '[data-action="ai-shorten"]', '[data-action="ai-expand"]', '[data-action="ai-formal"]', '[data-action="aiSettings"]'],
}

try:
    call('chrome_status')
    TAB = call('chrome_new_tab', url=URL)['tab']['id']
    (OUT / 'compact-tab-id.txt').write_text(TAB)
    wait_for('!!window.rustWord?.features')
    if evaluate("!document.querySelector('#modal').classList.contains('hidden')"):
        if evaluate("document.querySelector('#modal-title').textContent.includes('恢复')"):
            click('#modal-actions button:first-child')
        else:
            raise AssertionError('Unexpected isolated startup dialog')
    sample = {'type': 'doc', 'content': [
        {'type': 'heading', 'attrs': {'level': 1}, 'content': [{'type': 'text', 'text': '把时间留给写作'}]},
        {'type': 'paragraph', 'content': [{'type': 'text', 'text': '紧凑的工具栏，让常用操作触手可及，也为正文留下更多空间。'}]},
        {'type': 'heading', 'attrs': {'level': 2}, 'content': [{'type': 'text', 'text': '今天的工作安排'}]},
        {'type': 'paragraph', 'content': [{'type': 'text', 'text': '整理会议记录，确认本周计划，再完成一份清晰的工作报告。'}]},
        {'type': 'paragraph'}]}
    evaluate('window.rustWord.loadDocument(' + json.dumps({'document': sample, 'name': '写作工具栏预览'}, ensure_ascii=False) + ')')
    for width, height in [(1280, 900), (860, 650)]:
        call('chrome_cdp', method='Emulation.setDeviceMetricsOverride', params={'width': width, 'height': height, 'deviceScaleFactor': 1, 'mobile': False})
        for panel, commands in CORE.items():
            click('[data-tab="' + panel + '"]')
            selectors = ['.ribbon[data-panel="' + panel + '"] ' + command for command in commands]
            measured = evaluate(MEASURE + '(' + json.dumps(selectors) + ')')
            measured['panel'] = panel
            MATRIX.append(measured)
            label = f'{width}×{height} {panel}'
            check(abs(measured['totalHeight'] - 160) <= 1, label + ' 顶部三栏合计 160px', measured['parts'])
            clipped = [command for command in measured['commands'] if not command['visible']]
            check(not clipped, label + ' 核心按钮可见且未被容器裁剪', clipped)
            check(measured['workspaceTop'] <= 161, label + ' 正文区域从紧凑工具栏下方开始', measured['workspaceTop'])
            if panel in ('home', 'writing'):
                name = f'compact-{panel}.png' if width == 1280 else f'compact-{panel}-narrow.png'
                screenshot(name)

        # Open, close and reopen through real buttons at each width.
        click('.topbar [data-action="templates"]')
        wait_for("!document.querySelector('#template-library').classList.contains('hidden')")
        click('.template-card[data-template-id="builtin-meeting"]')
        measured = evaluate(MEASURE + "(['#template-close','#template-search','#template-use'])")
        check(all(c['visible'] for c in measured['commands']), f'{width}×{height} 模板搜索预览与使用按钮可达', measured['commands'])
        check(evaluate("document.querySelector('#template-preview-paper').textContent.includes('行动事项')"), f'{width}×{height} 模板正文预览仍真实渲染')
        click('#template-close')
        click('.topbar [data-action="templates"]')
        wait_for("!document.querySelector('#template-library').classList.contains('hidden')")
        check(evaluate("document.querySelectorAll('.template-card').length>=8"), f'{width}×{height} 模板库可重新打开')
        click('#template-close')
        click('.quick-actions [data-action="ai"]')
        wait_for("!document.querySelector('#ai-panel').classList.contains('hidden')")
        click('[data-ai-tab="rewrite"]')
        measured = evaluate(MEASURE + "(['#ai-close','#ai-settings','#ai-mode','#ai-generate','#ai-apply'])")
        check(all(c['visible'] for c in measured['commands']), f'{width}×{height} AI 入口模式生成与应用按钮可达', measured['commands'])
        # Test readability using user-editable synthetic text. This is not an AI
        # success mock: no request is sent and the Apply button remains disabled.
        text = '仅供排版验收：建议可以在这里检查与编辑，不代表本轮调用了 AI。'
        call('chrome_type', selector='#ai-result', text=text, clear=True)
        body = evaluate("(()=>{const r=document.querySelector('.ai-panel-body').getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2};})()")
        call('chrome_scroll', x=body['x'], y=body['y'], delta_y=1000)
        # At the minimum window height the two text fields do not both fit, but
        # each must become readable through the panel's own scroll container.
        result = evaluate("(()=>{const r=document.querySelector('#ai-result').getBoundingClientRect(),b=document.querySelector('.ai-panel-body').getBoundingClientRect();return {top:r.top,bottom:r.bottom,bodyTop:b.top,bodyBottom:b.bottom,value:document.querySelector('#ai-result').value,sourceReadonly:document.querySelector('#ai-source').readOnly,disabled:document.querySelector('#ai-apply').disabled};})()")
        check(result['top'] >= result['bodyTop'] - .5 and result['bottom'] <= result['bodyBottom'] + .5 and result['value'] == text and result['disabled'], f'{width}×{height} AI 建议可滚动阅读且未伪造生成成功', result)
        call('chrome_scroll', x=body['x'], y=body['y'], delta_y=-220)
        source = evaluate("(()=>{const r=document.querySelector('#ai-source').getBoundingClientRect(),b=document.querySelector('.ai-panel-body').getBoundingClientRect();return {top:r.top,bottom:r.bottom,bodyTop:b.top,bodyBottom:b.bottom,readonly:document.querySelector('#ai-source').readOnly};})()")
        check(source['readonly'] and source['top'] >= source['bodyTop'] - .5 and source['bottom'] <= source['bodyBottom'] + .5, f'{width}×{height} AI 原文区域可独立滚动阅读', source)
        if width == 860:
            screenshot('compact-ai-narrow.png')
        click('#ai-close')
        click('.quick-actions [data-action="ai"]')
        wait_for("!document.querySelector('#ai-panel').classList.contains('hidden')")
        click('[data-ai-tab="rewrite"]')
        check(evaluate("document.querySelector('#ai-close').getBoundingClientRect().right<=innerWidth"), f'{width}×{height} AI 面板可重新打开并关闭')
        click('#ai-close')
    evaluate("window.rustWord.invoke('discardRecovery')")
except Exception as error:
    ERROR = {'type': type(error).__name__, 'message': str(error)}
    print('ERROR ' + str(error), flush=True)
    traceback.print_exc()
    if TAB:
        try:
            screenshot('compact-failure.png')
        except Exception:
            pass
finally:
    if TAB:
        try:
            call('chrome_cdp', method='Emulation.clearDeviceMetricsOverride', params={})
            call('chrome_close_tab')
            CLOSED = True
        except Exception as error:
            if ERROR is None:
                ERROR = {'type': type(error).__name__, 'message': 'Close owned tab: ' + str(error)}
    (OUT / 'compact-checks.json').write_text(json.dumps({'url': URL, 'tab': TAB, 'passed': len(CHECKS), 'checks': CHECKS, 'measurements': MATRIX, 'issues': ISSUES, 'error': ERROR, 'tabClosed': CLOSED}, ensure_ascii=False, indent=2) + '\n')
if ERROR or ISSUES:
    raise SystemExit(1)
print(f'{len(CHECKS)} 项紧凑布局检查通过，8 组工具栏测量均已留证', flush=True)
