#!/usr/bin/env python3
"""Browser + Rust chat integration with a local deterministic model endpoint.

Uses a disposable profile, synthetic text and Playwright CLI. No user keys,
documents or running application state are read or modified.
"""
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'test-output' / 'chat'
OUT.mkdir(parents=True, exist_ok=True)
requests = []
failed_once = False
checks = []


class Model(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        global failed_once
        data = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        requests.append(data)
        message = data['messages'][-1]['content']
        if 'FAIL_ONCE' in message and not failed_once:
            failed_once = True
            self.send_response(503)
            self.end_headers()
            return
        if 'SLOW_CHECK' in message:
            time.sleep(2)
        answer = '# 对话测试\n\n这是 **可编辑** 的回答。\n\n- 连续追问\n- 插入文档\n\n<script>window.chatUnsafe=true</script>'
        body = json.dumps({'choices': [{'message': {'content': answer}}]}, ensure_ascii=False).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


cli_base = ['npx', '--yes', '--package', '@playwright/cli', 'playwright-cli', '--session', 'rust-word-chat-check']


def cli(*args):
    result = subprocess.run(cli_base + list(args), capture_output=True, text=True, timeout=90, cwd=ROOT)
    output = result.stdout + result.stderr
    if result.returncode or '### Error' in output:
        raise AssertionError(output[:2500])
    return output


def run(body):
    return cli('run-code', 'async (page) => {' + body + '}')


def check(name, body):
    run(body)
    checks.append(name)
    print('PASS ' + name, flush=True)


def rpc(command, **data):
    request = urllib.request.Request(url + '/rpc', json.dumps({'command': command, **data}).encode(), {'Content-Type': 'application/json'})
    result = json.load(urllib.request.urlopen(request, timeout=30))
    if 'error' in result:
        raise AssertionError(result['error'])
    return result['data']


done = 'await page.waitForFunction(()=>!window.rustWord.chat.isBusy());'
send = lambda text: 'await page.locator("#chat-input").fill(' + json.dumps(text) + ');await page.locator("#chat-send").click();' + done
reset = '''await page.locator('#chat-new').click();await page.locator('#modal-actions button.primary').click();'''
load = '''await page.evaluate(()=>window.rustWord.loadDocument({name:'Synthetic',document:{type:'doc',content:[{type:'paragraph',content:[{type:'text',text:'SECRET_DOCUMENT_CONTEXT'}]}]}}));'''
model = ThreadingHTTPServer(('127.0.0.1', 0), Model)
threading.Thread(target=model.serve_forever, daemon=True).start()
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0))
    port = listener.getsockname()[1]
url = 'http://127.0.0.1:' + str(port)
with tempfile.TemporaryDirectory(prefix='rust-word-chat-') as directory:
    profile = Path(directory)
    (profile / 'ui-preferences.json').write_text('{"locale":"zh-CN"}')
    server = subprocess.Popen([str(ROOT / 'target/release/rust-word'), '--test-server', '127.0.0.1:' + str(port), directory], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(50):
            try:
                rpc('aiSettings')
                break
            except OSError:
                time.sleep(.1)
        rpc('saveAiSettings', settings={'provider': 'openai', 'baseUrl': 'http://127.0.0.1:' + str(model.server_port) + '/v1', 'model': 'Local test', 'apiKey': 'synthetic-test-key'})
        cli('close')
        cli('open', url)
        cli('snapshot')
        run('await page.waitForFunction(()=>window.rustWord?.ready);' + load + 'await page.locator("[data-action=ai]").first().click();')
        check('direct chat without selection; Markdown remains safe', send('给我写一篇文章') + '''if(await page.locator('.chat-user').count()!==1)throw Error('missing user bubble');if(await page.locator('.chat-answer h1').count()!==1)throw Error('heading missing');if(await page.evaluate(()=>!!window.chatUnsafe))throw Error('HTML executed');if(await page.locator('.ProseMirror').innerText()!=='SECRET_DOCUMENT_CONTEXT')throw Error('document modified implicitly');''')
        assert [m['role'] for m in requests[-1]['messages']] == ['system', 'user']
        assert 'SECRET_DOCUMENT_CONTEXT' not in json.dumps(requests[-1])
        check('follow-up retains true user and assistant roles', send('写得更口语化一些') + '''if(await page.locator('.chat-answer').count()!==2)throw Error('missing follow-up');''')
        assert [m['role'] for m in requests[-1]['messages']] == ['system', 'user', 'assistant', 'user']
        check('direct service picker persists without clearing history or draft', """await page.locator('#chat-input').fill('保留我的草稿');for(const provider of ['workbuddy','codex','openai']){await page.locator('#chat-provider-select').selectOption(provider);await page.waitForFunction(()=>!window.rustWord.chat.isBusy());const saved=await page.evaluate(()=>window.rustWord.invoke('aiSettings'));if(saved.provider!==provider)throw Error('provider not saved');}if(await page.locator('#chat-input').inputValue()!=='保留我的草稿'||await page.locator('.chat-answer').count()!==2)throw Error('conversation lost');await page.locator('#chat-input').fill('');""")
        check('insert is editable, DOCX round-trips and one undo restores', '''const original=await page.evaluate(()=>JSON.stringify(window.rustWord.document()));await page.locator('.chat-message-actions').last().getByRole('button',{name:'插入文末',exact:true}).click();const doc=await page.evaluate(()=>window.rustWord.document());if(!doc.content.some(n=>n.type==='heading'))throw Error('not editable Markdown');await page.evaluate(doc=>window.rustWord.invoke('savePath',{path:'chat.docx',document:doc}),doc);const opened=await page.evaluate(()=>window.rustWord.invoke('openPath',{path:'chat.docx'}));const stable=x=>JSON.stringify(x,(_,v)=>v&&typeof v==='object'&&!Array.isArray(v)?Object.fromEntries(Object.entries(v).sort(([a],[b])=>a.localeCompare(b))):v);if(stable(doc)!==stable(opened.document))throw Error('DOCX differs');await page.evaluate(()=>window.rustWord.editor.commands.undo());if(await page.evaluate(()=>JSON.stringify(window.rustWord.document()))!==original)throw Error('undo failed');''')
        check('selected reference can replace and undo', reset + load + '''await page.evaluate(()=>window.rustWord.editor.commands.setTextSelection({from:1,to:7}));await page.locator('#chat-reference-selection').click();''' + send('修改这段') + '''const button=page.locator('[data-replace-turn]');if(await button.isDisabled())throw Error('replacement disabled');await button.click();if((await page.locator('.ProseMirror').innerText()).startsWith('SECRET'))throw Error('replacement failed');await page.evaluate(()=>window.rustWord.editor.commands.undo());if(await page.locator('.ProseMirror').innerText()!=='SECRET_DOCUMENT_CONTEXT')throw Error('replacement undo failed');''')
        assert 'Reference text (selection;' in requests[-1]['messages'][-1]['content']
        check('edits invalidate stale selection replacement', reset + load + '''await page.evaluate(()=>window.rustWord.editor.commands.setTextSelection({from:1,to:7}));await page.locator('#chat-reference-selection').click();''' + send('继续改写') + '''await page.evaluate(()=>window.rustWord.editor.commands.insertContent('changed'));if(!await page.locator('[data-replace-turn]').isDisabled())throw Error('stale answer can overwrite');''')
        check('whole-document reference is explicit', reset + load + '''await page.locator('#chat-reference-document').click();''' + send('总结全文'))
        assert 'SECRET_DOCUMENT_CONTEXT' in requests[-1]['messages'][-1]['content']
        check('Shift Enter and IME composition do not submit', '''await page.locator('#chat-input').fill('draft');await page.locator('#chat-input').press('Shift+Enter');await page.locator('#chat-input').dispatchEvent('keydown',{key:'Enter',isComposing:true});if(await page.evaluate(()=>window.rustWord.chat.isBusy()))throw Error('IME submitted');if(!(await page.locator('#chat-input').inputValue()).includes(String.fromCharCode(10)))throw Error('missing newline');''')
        check('panel reopen retains conversation and draft', '''const draft=await page.locator('#chat-input').inputValue();await page.locator('#ai-close').click();await page.locator('[data-action=ai]').first().click();if(await page.locator('#chat-input').inputValue()!==draft||await page.locator('.chat-answer').count()!==1)throw Error('chat lost');''')
        check('AI request leaves document service responsive', '''await page.locator('#chat-input').fill('SLOW_CHECK');await page.locator('#chat-send').click();await page.waitForFunction(()=>window.rustWord.chat.isBusy());if(!await page.locator('#chat-new').isDisabled()||!await page.locator('#chat-provider-select').isDisabled())throw Error('controls enabled while generating');const start=Date.now();await page.evaluate(()=>window.rustWord.invoke('autosave',{document:window.rustWord.document()}));if(Date.now()-start>1200)throw Error('document service blocked');''' + done)
        check('error remains visible and retry succeeds without duplicates', reset + send('FAIL_ONCE') + '''await page.locator('.chat-error').waitFor({state:'visible'});await page.locator('.chat-assistant').getByRole('button',{name:'重试',exact:true}).click();''' + done + '''if(await page.locator('.chat-user').count()!==1||await page.locator('.chat-answer').count()!==1)throw Error('retry duplicated history');''')
        check('language switch preserves chat and draft', '''await page.locator('#chat-input').fill('中文草稿');await page.locator('#language-select').selectOption('en');await page.locator('#modal-actions button.primary').click();await page.waitForFunction(()=>window.rustWord?.ready&&window.__rustWordBoot?.locale==='en');await page.locator('[data-action=ai]').first().click();if(await page.locator('#chat-input').inputValue()!=='中文草稿')throw Error('draft lost');if(await page.locator('.chat-answer').count()!==1)throw Error('history lost');if(await page.locator('#chat-send').innerText()!=='Send')throw Error('label untranslated');''')
        check('legacy selected-text rewrite remains available', '''await page.locator('[data-ai-tab=rewrite]').click();if(!await page.locator('#ai-generate').isVisible())throw Error('rewrite hidden');await page.evaluate(()=>window.rustWord.editor.commands.setTextSelection({from:1,to:7}));await page.locator('#ai-generate').click();await page.waitForFunction(()=>!document.querySelector('#ai-generate').disabled);if(await page.locator('#ai-apply').isDisabled())throw Error('rewrite failed');await page.locator('#ai-apply').click();await page.evaluate(()=>window.rustWord.editor.commands.undo());if(await page.locator('.ProseMirror').innerText()!=='SECRET_DOCUMENT_CONTEXT')throw Error('rewrite undo failed');await page.locator('[data-ai-tab=chat]').click();''')
        check('compact panel keeps composer visible', '''await page.setViewportSize({width:1000,height:700});const input=await page.locator('#chat-input').boundingBox();const send=await page.locator('#chat-send').boundingBox();if(!input||!send||send.y+send.height>700||input.width<200)throw Error('composer clipped');await page.screenshot({path:'test-output/chat/english-chat.png'});''')
        check('new conversation clears chat but retains document', '''const before=await page.locator('.ProseMirror').innerText();await page.locator('#chat-new').click();await page.locator('#modal-actions button.primary').click();if(await page.locator('.chat-user').count()!==0)throw Error('chat not cleared');if(before!==await page.locator('.ProseMirror').innerText())throw Error('document changed');''')
        (OUT / 'validation.json').write_text(json.dumps({'checks': checks, 'requests': len(requests)}, indent=2))
        print(json.dumps({'passed': len(checks)}))
    finally:
        cli('close')
        server.terminate()
        server.wait(timeout=10)
        model.shutdown()
