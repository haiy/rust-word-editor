"""Integration tests in the user's persistent, visible codex-chrome session.

Requires the Rust binary's isolated --test-server and codex-chrome-mcp env.
No credentials or personal documents are saved in test evidence.
"""
import base64
import json
import os
from pathlib import Path
import shlex
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "test-output"
OUT.mkdir(exist_ok=True)
(OUT / 'recovery.json').unlink(missing_ok=True)
config = dict(os.environ)
env_file = Path.home() / ".config/codex-chrome-mcp/env"
for line in env_file.read_text().splitlines():
    line = line.strip().removeprefix("export ")
    if not line or line.startswith("#") or "=" not in line:
        continue
    key, value = line.split("=", 1)
    tokens = shlex.split(value)
    if tokens:
        config.setdefault(key, tokens[0])
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
base = config["CODEX_CHROME_API_URL_LAN"]
tab = None


def call(name, **arguments):
    if tab and name != "chrome_new_tab":
        arguments.setdefault("tab_id", tab)
    request = urllib.request.Request(
        base + "/api/tools/call",
        data=json.dumps({"name": name, "arguments": arguments}).encode(),
        headers={"Authorization": "Bearer " + config["CODEX_CHROME_API_TOKEN"], "Content-Type": "application/json"},
    )
    result = json.load(opener.open(request, timeout=45))
    if not result.get("ok") or result.get("result", {}).get("isError"):
        raise RuntimeError(str(result))
    text = "".join(c.get("text", "") for c in result["result"]["content"] if c["type"] == "text")
    return json.loads(text)


def evaluate(expression):
    result = call("chrome_cdp", method="Runtime.evaluate", params={"expression": expression, "returnByValue": True, "awaitPromise": True})
    if result.get("exceptionDetails"):
        raise AssertionError(result["exceptionDetails"])
    return result.get("result", {}).get("value")


def click(selector):
    return call("chrome_click", selector=selector)


def text(selector, value):
    return call("chrome_type", selector=selector, text=value, clear=True)


checks = []


def check(condition, description):
    assert condition, description
    checks.append(description)
    print("PASS " + description, flush=True)


try:
    url = os.environ.get("WORD_TEST_URL", "http://127.0.0.1:18765/")
    tab = call("chrome_new_tab", url=url)["tab"]["id"]
    (OUT / 'tab-id.txt').write_text(tab)
    for _ in range(30):
        if evaluate("!!window.rustWord"):
            break
        time.sleep(.2)
    check(evaluate("!!window.rustWord"), "编辑器实际初始化")
    # Reused isolated servers can contain a draft from earlier synthetic tests.
    time.sleep(.4)
    if evaluate("!document.querySelector('#modal').classList.contains('hidden')"):
        assert '恢复' in evaluate("document.querySelector('#modal-title').textContent")
        click('#modal-actions button:first-child')
    click('[data-action="new"]')
    text('.ProseMirror', '中文实际输入：你好，Word！')
    check('中文实际输入' in evaluate('window.rustWord.editor.getText()'), "中文输入进入文档模型")
    call("chrome_keypress", key="Control+A")
    click('[data-action="bold"]')
    check(evaluate("window.rustWord.editor.getJSON().content[0].content[0].marks.some(m=>m.type==='bold')"), "工具栏加粗保留选择范围")
    click('[data-action="h1"]')
    check(evaluate("document.querySelectorAll('.outline-item').length") == 1, "标题生成真实章节导航")
    click('[data-action="undo"]')
    check(evaluate("window.rustWord.editor.getJSON().content[0].type") == 'paragraph', "撤销恢复文档结构")
    click('[data-action="redo"]')
    check(evaluate("window.rustWord.editor.getJSON().content[0].type") == 'heading', "重做恢复标题")

    # Additional rich content through the actual editor commands, then use actual
    # dialog/buttons for table insertion and find/replace.
    evaluate("window.rustWord.editor.chain().focus('end').insertContent([{type:'paragraph',content:[{type:'text',text:'查找目标，查找目标。'}]},{type:'paragraph'}]).run()")
    click('[data-tab="insert"]')
    click('[data-action="tableDialog"]')
    text('#table-rows', '3')
    text('#table-cols', '2')
    click('#modal-actions .primary')
    check(evaluate("document.querySelectorAll('.ProseMirror tr').length") == 3, "真实插入表格对话框")
    click('[data-action="addRowAfter"]')
    check(evaluate("document.querySelectorAll('.ProseMirror tr').length") == 4, "表格增行按钮有效")
    click('[data-action="deleteRow"]')
    check(evaluate("document.querySelectorAll('.ProseMirror tr').length") == 3, "表格删行按钮有效")
    evaluate("window.rustWord.editor.commands.insertContent('表格可以编辑')")
    check('表格可以编辑' in evaluate('window.rustWord.editor.getText()'), "表格单元格可以编辑")
    pixel = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/l3cAAAAASUVORK5CYII='
    evaluate("window.rustWord.editor.commands.focus('end')")
    evaluate("window.rustWord.insertImage(" + json.dumps('data:image/png;base64,' + pixel) + ", '集成测试图片')")
    check(evaluate("document.querySelector('.ProseMirror img')?.getAttribute('src').startsWith('data:image/png')"), "图片实际嵌入编辑区")
    click('[data-tab="layout"]')
    click('[data-action="landscape"]')
    check(evaluate("window.rustWord.editor.getJSON().attrs.orientation") == 'landscape', "页面方向进入文档模型")
    click('[data-action="find"]')
    text('#search', '查找目标')
    check(evaluate("document.querySelectorAll('.find-match').length") == 2, "查找高亮匹配正文")
    text('#replace', '替换完成')
    click('[data-action="replaceAll"]')
    check(evaluate("window.rustWord.editor.getText().includes('替换完成，替换完成。')"), "全部替换真实修改正文")
    click('#find-close')

    before = evaluate('window.rustWord.document()')
    evaluate("window.rustWord.invoke('savePath',{path:'ui-roundtrip.docx',document:window.rustWord.document()})")
    opened = evaluate("window.rustWord.invoke('openPath',{path:'ui-roundtrip.docx'})")
    check(opened['document'] == before, "真实 Rust DOCX 保存与重开，完整文档模型一致")
    evaluate("window.rustWord.invoke('openPath',{path:'ui-roundtrip.docx'}).then(r=>window.rustWord.loadDocument(r))")
    text('.ProseMirror', '重开后还能再次编辑中文。')
    check('重开后还能再次编辑中文' in evaluate('window.rustWord.editor.getText()'), "重开后继续输入编辑")
    evaluate("window.rustWord.invoke('autosave',{document:window.rustWord.document(),savedAt:new Date().toISOString()})")
    recovery = evaluate("window.rustWord.invoke('ready')")
    check('重开后还能再次编辑中文' in json.dumps(recovery['recovery'], ensure_ascii=False), "恢复草稿保存到真实文件")
    evaluate("window.rustWord.invoke('discardRecovery')")

    # Foreign Word document created with an independent Office library.
    if (OUT / 'foreign.docx').exists():
        foreign = evaluate("window.rustWord.invoke('openPath',{path:'foreign.docx'})")
        flat = json.dumps(foreign['document'], ensure_ascii=False)
        check('外部 Word 标题' in flat and '正常文字' in flat and '合并单元格' in flat, "外部 DOCX 导入正文及表格")
        check(foreign['protected'] and foreign['warnings'], "不支持的页眉触发原件保护")
        protected = evaluate("window.rustWord.invoke('savePath',{path:'foreign.docx',document:window.rustWord.document()}).then(()=>false,()=>true)")
        check(protected, "不能覆盖含不支持内容的原件")
        evaluate("window.rustWord.invoke('openPath',{path:'foreign.docx'}).then(r=>window.rustWord.loadDocument(r))")
        check(evaluate("window.rustWord.editor.getJSON().attrs.orientation") == 'portrait', "切换文件恢复各自纸张方向")
        check(evaluate("document.querySelectorAll('.outline-item').length") >= 1, "外部 Word 标题样式生成导航")

    capture = call('chrome_cdp', method='Page.captureScreenshot', params={'format':'png'})
    (OUT / 'editor-chrome.png').write_bytes(base64.b64decode(capture['data']))
    (OUT / 'editor-checks.json').write_text(json.dumps({'url':url,'tab':tab,'checks':checks,'passed':len(checks)},ensure_ascii=False,indent=2))
    print(f'{len(checks)} 项真实浏览器检查通过；证据位于 {OUT}')
finally:
    if tab:
        call("chrome_close_tab")
