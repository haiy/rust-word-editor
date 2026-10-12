"""PPT acceptance against real persistent codex-chrome and isolated Rust server.

Only synthetic content is used. Every successful export/AI response comes from
the real Rust backend. The owned browser tab is closed in finally. Native apps,
existing browser tabs, cloud accounts and original WPS templates are untouched.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import shlex
import struct
import time
import traceback
import urllib.request
import xml.etree.ElementTree as ET
import zipfile
import zlib

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'test-output'
OUT.mkdir(exist_ok=True)
URL = os.environ.get('WORD_TEST_URL', 'http://127.0.0.1:18769/')
SERVER_ROOT = OUT / ('server-' + URL.split(':')[-1].strip('/'))
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
ARTIFACTS = []
ERROR = None
CLOSED = False
START = time.monotonic()
# A repaired document-AI contract can be checked without repeating an already
# successful paid/limited topic request. The previous run is kept as evidence;
# this mode resumes only verified successes, never synthesizes an AI response.
RESUME_DOCUMENT = os.environ.get('PPT_RESUME_DOCUMENT') == '1'
PREVIOUS = None
if RESUME_DOCUMENT:
    PREVIOUS = json.loads((OUT / 'ppt-checks.json').read_text())
    assert PREVIOUS.get('tabClosed') and len(PREVIOUS.get('aiResults', [])) == 1
    assert PREVIOUS['aiResults'][0]['action'] == 'pptGenerate'
    assert PREVIOUS['aiResults'][0]['deck']['provider'] == 'codex'
    (OUT / 'ppt-initial-checks.json').write_text(json.dumps(PREVIOUS, ensure_ascii=False, indent=2) + '\n')
    (OUT / 'ppt-initial-run.log').write_text((OUT / 'ppt-run.log').read_text())
    AI_RESULTS.extend(PREVIOUS['aiResults'])
    ARTIFACTS.extend(PREVIOUS.get('artifacts', []))
    CHECKS.extend(c for c in PREVIOUS['checks'] if not c.startswith('pptFromDocument'))


def call(name, **arguments):
    if TAB and name != 'chrome_new_tab':
        arguments.setdefault('tab_id', TAB)
    request = urllib.request.Request(BASE + '/api/tools/call',
        data=json.dumps({'name': name, 'arguments': arguments}).encode(),
        headers={'Authorization': 'Bearer ' + CONFIG['CODEX_CHROME_API_TOKEN'], 'Content-Type': 'application/json'})
    result = json.load(OPENER.open(request, timeout=44))
    if not result.get('ok') or result.get('result', {}).get('isError'):
        raise RuntimeError(str(result))
    text = ''.join(b.get('text', '') for b in result['result']['content'] if b['type'] == 'text')
    return json.loads(text)


def evaluate(expression):
    result = call('chrome_cdp', method='Runtime.evaluate', params={'expression': expression, 'returnByValue': True, 'awaitPromise': True})
    if result.get('exceptionDetails'):
        raise AssertionError(result['exceptionDetails'])
    return result.get('result', {}).get('value')


def click(selector):
    call('chrome_click', selector=selector)


def type_text(selector, value):
    if value == '':
        click(selector)
        call('chrome_keypress', key='Control+A')
        call('chrome_keypress', key='Backspace')
    else:
        call('chrome_type', selector=selector, text=value, clear=True)


def select_option(selector, value):
    index = evaluate(f"[...document.querySelector({json.dumps(selector)}).options].findIndex(o=>o.value==={json.dumps(value)})")
    assert index >= 0
    click(selector)
    call('chrome_keypress', key='Home')
    for _ in range(index):
        call('chrome_keypress', key='ArrowDown')
    call('chrome_keypress', key='Enter')
    assert evaluate(f"document.querySelector({json.dumps(selector)}).value") == value


def wait_for(expression, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = evaluate(expression)
        if value:
            return value
        time.sleep(.4)
    raise AssertionError('Timeout: ' + expression)


def check(condition, description):
    if not condition:
        raise AssertionError(description)
    if description not in CHECKS:
        CHECKS.append(description)
    print('PASS ' + description, flush=True)


def screenshot(name):
    data = call('chrome_cdp', method='Page.captureScreenshot', params={'format': 'png'})
    (OUT / name).write_bytes(base64.b64decode(data['data']))


def deck():
    return evaluate('window.rustWord.ppt.getDeck()')


def word():
    return evaluate('window.rustWord.editor.getJSON()')


def export_native(name, expected):
    result = evaluate("(async()=>{const r=await window.rustWord.invoke('pptExport',{deck:window.rustWord.ppt.getDeck()});const saved=await window.rustWord.invoke('saveBytesPath',{data:r.data,name:r.name,path:" + json.dumps(name) + "});return {bytes:r.bytes,name:r.name,saved};})()")
    path = SERVER_ROOT / name
    check(path.is_file() and path.stat().st_size == result['bytes'], name + ' 由 Rust 实际导出并写入隔离磁盘')
    with zipfile.ZipFile(path) as archive:
        check(archive.testzip() is None, name + ' ZIP 校验通过')
        for file in archive.namelist():
            if file.endswith(('.xml', '.rels')):
                ET.fromstring(archive.read(file))
        check(True, name + ' 所有 XML 与关系文件可解析')
        presentation = ET.fromstring(archive.read('ppt/presentation.xml'))
        ns = {'p': 'http://schemas.openxmlformats.org/presentationml/2006/main', 'a': 'http://schemas.openxmlformats.org/drawingml/2006/main'}
        size = presentation.find('p:sldSz', ns)
        check(size.attrib['cx'] == '12192000' and size.attrib['cy'] == '6858000', name + ' 输出宽屏 16:9')
        check(len(presentation.findall('p:sldIdLst/p:sldId', ns)) == len(expected['slides']), name + ' 页数与编辑模型一致')
        for i, slide in enumerate(expected['slides']):
            root = ET.fromstring(archive.read(f'ppt/slides/slide{i+1}.xml'))
            texts = [t.text or '' for t in root.findall('.//a:t', ns)]
            check(slide['title'] in texts and all(p in texts for p in slide['body']), f'{name} 第 {i+1} 页标题正文是真实原生 a:t')
            check(not root.findall('.//p:pic', ns) and len(root.findall('.//p:sp', ns)) >= 3, f'{name} 第 {i+1} 页保留可编辑形状文字')
        embedded = json.loads(archive.read('ppt/rustWord.json'))
        check(embedded == expected, name + ' 无损编辑模型保存在 PPTX')
    reopened = evaluate("(async()=>{const file=await window.rustWord.invoke('pickBytesPath',{path:" + json.dumps(name) + "});return await window.rustWord.invoke('pptImport',{data:file.data});})()")
    check(reopened == expected, name + ' 由磁盘真实重读并导入完整模型')
    evaluate('window.rustWord.ppt.setDeck(' + json.dumps(reopened, ensure_ascii=False) + ',false)')
    check(deck() == expected and not evaluate('window.rustWord.ppt.isDirty()'), name + ' 重开后逐页可编辑且为已保存状态')
    ARTIFACTS.append({'path': str(path), 'bytes': path.stat().st_size, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})


def generate_ai(from_document, topic, before):
    # Use only genuine UI commands. No request promise is awaited in CDP: each
    # status poll is an independent short call while Codex runs asynchronously.
    evaluate('window.rustWord.ppt.setDeck(' + json.dumps(deck(), ensure_ascii=False) + ',false)')
    click('#ppt-close')
    click('[data-tab="tools"]')
    action = 'pptFromDocument' if from_document else 'pptGenerate'
    click('[data-panel="tools"] [data-action="' + action + '"]')
    wait_for("!document.querySelector('#modal').classList.contains('hidden')&&!!document.querySelector('#ppt-topic')")
    type_text('#ppt-topic', topic)
    type_text('#ppt-count', '3')
    check(evaluate("document.querySelector('#ppt-topic').value") == topic and evaluate("document.querySelector('#ppt-count').value") == '3', action + ' 真实弹窗输入主题及页数')
    started = time.monotonic()
    click('#modal-actions button:last-child')
    check(word() == before, action + ' 生成请求启动后 Word 原文未改变')
    deadline = time.monotonic() + 620
    last_note = 0
    while time.monotonic() < deadline:
        state = evaluate("({busy:document.querySelector('#ppt-overlay').classList.contains('busy'),status:document.querySelector('#ppt-status').textContent,error:document.querySelector('#ppt-status').classList.contains('error'),editor:!!document.querySelector('#ppt-slide-title')})")
        if not state['busy']:
            if state['error'] or not state['editor'] or 'AI 大纲已生成' not in state['status']:
                raise AssertionError(action + ': ' + json.dumps(state, ensure_ascii=False))
            break
        elapsed = time.monotonic() - started
        if elapsed - last_note >= 25:
            print(f'WAIT {action}: {elapsed:.0f}s {state["status"]}', flush=True)
            last_note = elapsed
        time.sleep(2)
    else:
        raise AssertionError(action + ' AI generation exceeded bounded wait')
    generated = deck()
    check(generated.get('provider') == 'codex', action + ' 返回真实 Codex provider')
    check(len(generated['slides']) == 3 and all(s['title'].strip() and 2 <= len(s['body']) <= 6 for s in generated['slides']), action + ' 实际生成 3 页有内容的大纲')
    check(word() == before, action + ' 完成后 Word 原文与格式结构未改变')
    first = generated['slides'][0]['title']
    type_text('#ppt-slide-title', first + ' · 已检查')
    check(deck()['slides'][0]['title'] == first + ' · 已检查', action + ' 真实 AI 页面可以继续编辑')
    screenshot('ppt-ai-document.png' if from_document else 'ppt-ai-topic.png')
    AI_RESULTS.append({'action': action, 'topic': topic, 'elapsedSeconds': round(time.monotonic()-started, 2), 'deck': deck(), 'wordUnchanged': True})
    export_native('ppt-ai-document.pptx' if from_document else 'ppt-ai-topic.pptx', deck())


try:
    call('chrome_status')
    TAB = call('chrome_new_tab', url=URL)['tab']['id']
    (OUT / 'ppt-tab-id.txt').write_text(TAB)
    call('chrome_cdp', method='Emulation.setDeviceMetricsOverride', params={'width': 1440, 'height': 1000, 'deviceScaleFactor': 1, 'mobile': False})
    wait_for('!!window.rustWord?.ppt&&!!window.rustWord?.suite')
    if evaluate("!document.querySelector('#modal').classList.contains('hidden')"):
        if evaluate("document.querySelector('#modal-title').textContent.includes('恢复')"):
            click('#modal-actions button:first-child')
        else:
            raise AssertionError('Unexpected isolated startup dialog')
    source = {'type': 'doc', 'content': [
        {'type': 'heading', 'attrs': {'level': 1}, 'content': [{'type': 'text', 'text': '合成培训计划'}]},
        {'type': 'paragraph', 'content': [{'type': 'text', 'text': '本周组织一次团队写作培训，目标是把会议纪要写清楚，练习提炼行动项。', 'marks': [{'type': 'bold'}]}]},
        {'type': 'paragraph', 'content': [{'type': 'text', 'text': '培训安排：先演示记录方法，再分组整理一份合成纪要，最后核对负责人和完成时间。'}]},
        {'type': 'paragraph', 'content': [{'type': 'text', 'text': '课后每人完成一份练习纪要，由同伴反馈清晰度；不使用真实客户或员工信息。'}]}]}
    evaluate('window.rustWord.loadDocument(' + json.dumps({'document': source, 'name': 'PPT合成验收Word'}, ensure_ascii=False) + ')')
    source = word()
    settings = evaluate("window.rustWord.invoke('aiSettings')")
    check(settings['provider'] == 'codex' and 'apiKey' not in settings, '隔离服务使用真实 Codex，公开配置不暴露密钥')
    if not RESUME_DOCUMENT:
        click('[data-tab="tools"]')
        click('[data-panel="tools"] [data-action="pptTemplates"]')
        wait_for("!!document.querySelector('#ppt-library-kind')&&!document.querySelector('#ppt-overlay').classList.contains('busy')")
        cards = evaluate("[...document.querySelectorAll('.ppt-template-card')].map(b=>({id:b.dataset.pptTemplate,name:b.querySelector('strong').textContent,category:b.querySelector('small').textContent,svg:!!b.querySelector('svg')}))")
        check(len(cards) == 6 and len({c['id'] for c in cards}) == 6, '内容模板实际显示 6 个原创结构')
        check({c['category'] for c in cards} == {'汇报', '计划', '教学', '介绍', '通用'} and all(c['svg'] for c in cards), '原创结构分类与真实 SVG 缩略图完整')
        for card in cards:
            click('[data-ppt-template="' + card['id'] + '"]')
            check(evaluate("document.querySelector('#ppt-preview-title').textContent") == card['name'] and evaluate("!!document.querySelector('#ppt-template-preview svg text')"), card['name'] + ' 真实选择更新模板预览')
        type_text('#ppt-search', '培训')
        check(evaluate("document.querySelectorAll('.ppt-template-card').length") == 1 and evaluate("document.querySelector('#ppt-preview-title').textContent") == '培训课程', '搜索培训实际筛选结构与预览')
        type_text('#ppt-search', '不存在的合成模板词')
        check(evaluate("document.querySelectorAll('.ppt-template-card').length") == 0 and evaluate("document.querySelector('#ppt-template-use').disabled"), '零匹配搜索禁用使用按钮')
        type_text('#ppt-search', '')
        select_option('#ppt-library-kind', 'theme')
        check(evaluate("document.querySelectorAll('.ppt-template-card').length") == 3, '主题分类实际列出 3 个本机主题')
        theme_rows = evaluate("[...document.querySelectorAll('.ppt-template-card')].map(b=>({id:b.dataset.pptTemplate,name:b.querySelector('strong').textContent,category:b.querySelector('small').textContent}))")
        check(len({t['id'] for t in theme_rows}) == 3 and all(t['category'] == '本机主题' for t in theme_rows), '3 个主题标识唯一并保持本机主题分类')
        type_text('#ppt-search', 'Clear Blue')
        check(evaluate("document.querySelectorAll('.ppt-template-card').length") == 1, '主题搜索准确筛选 Clear Blue')
        check(evaluate("document.querySelector('#ppt-preview-title').textContent") == 'Clear Blue' and evaluate("!!document.querySelector('#ppt-template-preview svg')"), '本机主题真实配色预览可用')
        type_text('#ppt-search', '')
        select_option('#ppt-library-kind', 'structure')
        click('[data-ppt-template="training"]')
        screenshot('ppt-library.png')
        click('#ppt-template-use')
        wait_for("!!document.querySelector('#ppt-slide-title')&&!document.querySelector('#ppt-overlay').classList.contains('busy')")
        check(deck()['title'] == '培训课程' and len(deck()['slides']) == 4, '真实使用模板新建四页可编辑演示')
        check(word() == source, '使用 PPT 模板保留 Word 原始正文及格式')
        type_text('#ppt-deck-title', '合成验收 · 可编辑演示')
        for i in range(4):
            click('[data-slide-index="' + str(i) + '"]')
            title = f'第{i+1}页独立标题 & 编辑'
            body = [f'第{i+1}页第一项真实内容', f'第{i+1}页第二项 <保留中文与符号>']
            type_text('#ppt-slide-title', title)
            type_text('#ppt-slide-body', '\n'.join(body))
            current = deck()['slides'][i]
            check(current['title'] == title and current['body'] == body, f'第{i+1}页真实输入保存独立标题正文')
            check(evaluate("document.querySelector('#ppt-slide-preview').textContent.includes(" + json.dumps(body[1], ensure_ascii=False) + ")"), f'第{i+1}页编辑结果即时渲染')
        edited = deck()
        click('#ppt-add')
        check(len(deck()['slides']) == 5 and deck()['slides'][4]['title'] == '新页面', '添加按钮在所选页后创建新页')
        type_text('#ppt-slide-title', '测试重排页')
        select_option('#ppt-layout', 'section')
        check(deck()['slides'][4]['layout'] == 'section', '真实版式选择切换章节页')
        click('#ppt-up')
        check(deck()['slides'][3]['title'] == '测试重排页' and deck()['slides'][4]['id'] == edited['slides'][3]['id'], '上移交换相邻页并保留原页面身份')
        click('#ppt-down')
        check(deck()['slides'][4]['title'] == '测试重排页', '下移恢复页面顺序')
        click('#ppt-delete')
        check(deck()['slides'] == edited['slides'], '删除新页后四页编辑结果保持完整')
        themes = evaluate("[...document.querySelector('#ppt-theme').options].map(o=>({id:o.value,name:o.textContent}))")
        old = deck()['themeId']
        changed = next(t['id'] for t in themes if t['id'] != old)
        select_option('#ppt-theme', changed)
        check(deck()['themeId'] == changed and deck()['slides'] == edited['slides'], '主题切换修改主题并保留全部页面内容')
        check(evaluate("document.querySelector('#ppt-slide-preview svg').getAttribute('viewBox')") == '0 0 960 540', '编辑预览保持真实 16:9 SVG')
        screenshot('ppt-editor.png')
        final = deck()
        export_native('ppt-editable.pptx', final)
        images = evaluate("(async()=>{const deck=window.rustWord.ppt.getDeck();const images=[];for(let i=0;i<deck.slides.length;i++)images.push(await window.rustWord.ppt.rasterize(deck.slides[i],i));const r=await window.rustWord.invoke('pptExportImages',{images});return await window.rustWord.invoke('saveBytesPath',{data:r.data,path:'ppt-images.zip',name:'ppt-images.zip'});})()")
        image_path = SERVER_ROOT / 'ppt-images.zip'
        check(image_path.is_file() and image_path.stat().st_size == images['bytes'], '实际 Canvas 逐页渲染经 Rust 打包保存图片 ZIP')
        hashes = []
        with zipfile.ZipFile(image_path) as archive:
            check(archive.namelist() == [f'slide-{i+1:02}.png' for i in range(4)] and archive.testzip() is None, '图片 ZIP 含四页顺序正确且校验通过')
            for i, name in enumerate(archive.namelist()):
                data = archive.read(name)
                check(data[:8] == b'\x89PNG\r\n\x1a\n' and struct.unpack('>II', data[16:24]) == (1920,1080), f'图片第{i+1}页是真实 1920×1080 PNG')
                offset = 8
                compressed = b''
                while offset < len(data):
                    length = struct.unpack('>I', data[offset:offset+4])[0]
                    kind = data[offset+4:offset+8]
                    if kind == b'IDAT':
                        compressed += data[offset+8:offset+8+length]
                    offset += length + 12
                raw = zlib.decompress(compressed)
                check(len(set(raw)) > 20 and len(data) > 15000, f'图片第{i+1}页含真实渲染文字图形而非空白占位')
                hashes.append(hashlib.sha256(data).hexdigest())
                (OUT / ('ppt-' + name)).write_bytes(data)
        check(len(set(hashes)) == 4, '四页图片内容彼此不同并对应独立编辑页面')
        ARTIFACTS.append({'path': str(image_path), 'bytes': image_path.stat().st_size, 'sha256': hashlib.sha256(image_path.read_bytes()).hexdigest()})
        check(word() == source, 'PPT 编辑导出重开和转图片未修改 Word 原文')
    if RESUME_DOCUMENT:
        previous_topic = AI_RESULTS[0]['deck']
        # Re-import the first run's actual artifact and compare with its stored
        # successful result; no setDeck is used as a substitute for generation.
        topic_file = SERVER_ROOT / 'ppt-ai-topic.pptx'
        check(topic_file.is_file(), '首次真实主题 AI 导出文件仍保存在隔离磁盘')
        imported = evaluate("(async()=>{const f=await window.rustWord.invoke('pickBytesPath',{path:'ppt-ai-topic.pptx'});return await window.rustWord.invoke('pptImport',{data:f.data});})()")
        check(imported == previous_topic, '修复验收复用已验证的真实主题 AI 文件且模型完全一致')
        evaluate('window.rustWord.ppt.setDeck(' + json.dumps(imported, ensure_ascii=False) + ',false)')
    else:
        generate_ai(False, '合成演示：团队会议纪要怎么写。分为记录事实、明确行动项、检查负责人三个要点，不使用统计数字。', source)
        generated = deck()
        check(all('合成验收' not in s['title'] for s in generated['slides']), '主题 AI 生成真实替换原演示而非保留测试内容')
    generate_ai(True, '把这份合成培训计划做成三页团队学习演示，忠实概括已有内容。', source)
    combined = ' '.join(s['title'] + ' ' + ' '.join(s['body']) for s in deck()['slides'])
    check('培训' in combined and any(t in combined for t in ['纪要','行动项','负责人']), '文档 AI 实际使用当前 Word 的合成培训内容')
    evaluate("window.rustWord.invoke('discardRecovery')")
except Exception as error:
    ERROR = {'type': type(error).__name__, 'message': str(error)}
    print('ERROR ' + str(error), flush=True)
    traceback.print_exc()
    if TAB:
        try:
            screenshot('ppt-failure.png')
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
    (OUT / 'ppt-checks.json').write_text(json.dumps({'url': URL, 'tab': TAB, 'passed': len(CHECKS), 'checks': CHECKS, 'aiResults': AI_RESULTS, 'artifacts': ARTIFACTS, 'error': ERROR, 'tabClosed': CLOSED, 'elapsedSeconds':round(time.monotonic()-START,2)}, ensure_ascii=False, indent=2) + '\n')
if ERROR:
    raise SystemExit(1)
print(f'{len(CHECKS)} 项 PPT 独立检查通过，两次真实 Codex 生成，所有导出文件留证', flush=True)
