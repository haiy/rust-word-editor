#!/usr/bin/env python3
"""Exercise local file selection/drop in a running isolated browser preview.

Start `rust-word --test-server 127.0.0.1:PORT PROFILE` with an empty profile,
then run this script with --url and --profile. Requires Node/npx and Chrome.
Optional --document paths stay local and are never copied into the repository.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import urllib.request
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument('--url', required=True)
parser.add_argument('--profile', type=Path, required=True)
parser.add_argument('--session', default='rust-word-local-import')
parser.add_argument('--document', type=Path, action='append', default=[])
args = parser.parse_args()
args.profile = args.profile.resolve()
args.profile.mkdir(parents=True, exist_ok=True)
cli_base = ['npx', '--yes', '--package', '@playwright/cli', 'playwright-cli', '-s=' + args.session]
checks = []


def cli(*command):
    result = subprocess.run(cli_base + list(command), capture_output=True, text=True, timeout=90)
    output = result.stdout + result.stderr
    if result.returncode or '### Error' in output:
        raise RuntimeError('Browser check failed: ' + output[:1800])
    return output


def run(body):
    return cli('run-code', 'async (page) => {' + body + '}')


def check(label, body):
    run(body)
    checks.append(label)
    print('PASS ' + label, flush=True)


def rpc(command, **data):
    req = urllib.request.Request(args.url + '/rpc', json.dumps({'command': command, **data}).encode(), {'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=30) as response:
        result = json.load(response)
    if 'error' in result:
        raise RuntimeError(result['error'])
    return result['data']


def select(path):
    return 'await page.locator("#open-file-input").setInputFiles(' + json.dumps(str(path)) + ');'


wait = 'await page.waitForFunction(()=>!document.querySelector("#file-progress").classList.contains("hidden"));'
finished = 'await page.waitForFunction(()=>document.querySelector("#file-progress").classList.contains("hidden"));'
fixture = args.profile / 'synthetic-import.docx'
with zipfile.ZipFile(fixture, 'w', zipfile.ZIP_DEFLATED) as archive:
    archive.writestr('[Content_Types].xml', '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>')
    archive.writestr('word/document.xml', '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Synthetic research report</w:t></w:r></w:p><w:p><w:r><w:t>Local import works. 中文正文保持原样。</w:t></w:r></w:p></w:body></w:document>')
broken = args.profile / 'broken.docx'
broken.write_bytes(b'not a zip')
notes = args.profile / 'notes.txt'
notes.write_text('Imported text\n第二行', encoding='utf-8')
rpc('new')
cli('close')
cli('open', args.url)
run('await page.waitForFunction(()=>window.rustWord?.ready);')
# The real Open button must raise the browser file chooser.
output = run('await page.locator("[data-action=open]").click();')
if 'file chooser' not in output.lower():
    raise RuntimeError('Open did not show a browser file chooser')
cli('upload', str(fixture))
check('browser Open renders editable DOCX', 'await page.waitForFunction(()=>document.querySelector("#document-name").textContent==="synthetic-import.docx");' + finished + 'if(await page.locator(".ProseMirror").getAttribute("contenteditable")!=="true")throw Error("not editable");if(!await page.locator(".ProseMirror").innerText().then(t=>t.includes("中文正文保持原样")))throw Error("missing content");')
# Delay the actual request to observe in-progress UI, then continue to Rust.
check('visible progress and same-file reselection', '''await page.route('**/rpc',async route=>{const req=route.request().postDataJSON();if(req.command==='openFile')await new Promise(r=>setTimeout(r,500));await route.continue();});''' + select(fixture) + wait + '''if(await page.locator('.ProseMirror').getAttribute('contenteditable')!=='false')throw Error('editor not locked');''' + finished + '''await page.unroute('**/rpc');''')
check('broken DOCX shows persistent error and preserves content', 'const before=await page.locator(".ProseMirror").innerText();' + select(broken) + 'await page.locator("#file-open-error").waitFor({state:"visible"});' + finished + 'await page.waitForTimeout(4400);if(!await page.locator("#file-open-error").isVisible())throw Error("error vanished");if(before!==await page.locator(".ProseMirror").innerText())throw Error("content lost");')
check('network failure stays visible and preserves content', '''const before=await page.locator('.ProseMirror').innerText();await page.route('**/rpc',async route=>{if(route.request().postDataJSON().command==='openFile')await route.abort();else await route.continue();});''' + select(fixture) + '''await page.locator('#file-open-error').waitFor({state:'visible'});''' + finished + '''await page.unroute('**/rpc');if(before!==await page.locator('.ProseMirror').innerText())throw Error('content lost');''')
check('cancel preserves unsaved edits', '''await page.locator('.ProseMirror').press('ControlOrMeta+End');await page.locator('.ProseMirror').pressSequentially(' UNSAVED CHECK');''' + select(notes) + '''await page.locator('#modal').waitFor({state:'visible'});await page.locator('#modal-actions button').filter({hasText:/^Cancel$/}).click();if(!await page.locator('.ProseMirror').innerText().then(t=>t.includes('UNSAVED CHECK')))throw Error('edits lost');''')
check('UTF-8 text opens after discard', select(notes) + '''await page.locator('#modal').waitFor({state:'visible'});await page.locator('#modal-actions button').filter({hasText:/Don.t save/}).click();await page.waitForFunction(()=>document.querySelector('#document-name').textContent==='notes.txt');''' + finished + '''if(!await page.locator('.ProseMirror').innerText().then(t=>t.includes('第二行')))throw Error('text missing');''')
encoded = base64.b64encode(fixture.read_bytes()).decode()
check('DOCX drop uses real Rust parser', '''const transfer=await page.evaluateHandle(data=>{const dt=new DataTransfer();dt.items.add(new File([Uint8Array.from(atob(data),c=>c.charCodeAt(0))],'dropped.docx',{type:'application/vnd.openxmlformats-officedocument.wordprocessingml.document'}));return dt;},''' + json.dumps(encoded) + ''');await page.locator('.ProseMirror').dispatchEvent('drop',{dataTransfer:transfer});await page.waitForFunction(()=>document.querySelector('#document-name').textContent==='dropped.docx');''' + finished + '''if(await page.locator('#file-open-error').isVisible())throw Error('stale error');''')
check('edited import saves and reopens with identical content', '''await page.locator('.ProseMirror').press('ControlOrMeta+End');await page.locator('.ProseMirror').pressSequentially(' SAVED CHECK');const doc=await page.evaluate(()=>window.rustWord.document());const saved=await page.evaluate(doc=>window.rustWord.invoke('savePath',{path:'roundtrip.docx',document:doc}),doc);const reopened=await page.evaluate(()=>window.rustWord.invoke('openPath',{path:'roundtrip.docx'}));const stable=x=>JSON.stringify(x,(_,v)=>v&&typeof v==='object'&&!Array.isArray(v)?Object.fromEntries(Object.entries(v).sort(([a],[b])=>a.localeCompare(b))):v);if(stable(doc)!==stable(reopened.document))throw Error('roundtrip mismatch');await page.evaluate(r=>window.rustWord.loadDocument(r),reopened);''')
for action in ['save', 'saveAs']:
    output_path = args.profile / (action + '-download.docx')
    check('browser ' + action + ' downloads editable DOCX', "const [download]=await Promise.all([page.waitForEvent('download'),page.locator('[data-action=" + action + "]').click()]);await download.saveAs(" + json.dumps(str(output_path)) + ");await page.waitForFunction(()=>document.querySelector('#save-state').textContent==='Download started');")
    downloaded = rpc('importDocument', name='download.docx', data=base64.b64encode(output_path.read_bytes()).decode())
    expected = rpc('openPath', path='roundtrip.docx')
    if downloaded['document'] != expected['document']:
        raise RuntimeError('Downloaded DOCX content differs')
for index, path in enumerate(args.document, 1):
    before = hashlib.sha256(path.read_bytes()).hexdigest()
    # Compare actual editor text with the Rust import result without exposing it.
    expected = rpc('importDocument', name=path.name, data=base64.b64encode(path.read_bytes()).decode())['document']
    def texts(node):
        return ([node['text']] if node.get('type') == 'text' else []) + [text for child in node.get('content', []) for text in texts(child)]
    text = ''.join(texts(expected))
    check('private document ' + str(index) + ' renders complete text and accepts edits', select(path) + 'await page.waitForFunction(name=>document.querySelector("#document-name").textContent===name,' + json.dumps(path.name) + ');' + finished + 'const expected=' + json.dumps(text, ensure_ascii=False) + ';const actual=await page.locator(".ProseMirror").innerText();if(actual.replace(/\\s/g,"")!==expected.replace(/\\s/g,""))throw Error("imported text differs");await page.locator(".ProseMirror").press("ControlOrMeta+Home");await page.locator(".ProseMirror").pressSequentially("EDIT CHECK ");if(!await page.locator(".ProseMirror").innerText().then(t=>t.startsWith("EDIT CHECK ")))throw Error("editing failed");await page.evaluate(()=>window.rustWord.editor.commands.undo());')
    # Mark only the isolated preview clean; no write to the source file.
    run('await page.evaluate(()=>window.rustWord.loadDocument({document:window.rustWord.document(),name:"Verified copy"}));')
    if hashlib.sha256(path.read_bytes()).hexdigest() != before:
        raise RuntimeError('Original changed')
(args.profile / 'import-validation.json').write_text(json.dumps({'checks': checks, 'originals_unchanged': True}, indent=2))
print(json.dumps({'passed': len(checks), 'originals_unchanged': True}))
cli('close')
