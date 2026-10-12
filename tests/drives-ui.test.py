"""Real multi-drive UI and rclone against an isolated WebDAV protocol fixture.

Uses an owned persistent Chrome tab, real HTTP requests and real DOCX bytes.
The synthetic rclone config never reads or changes the user's authorization.
"""
import base64, hashlib, json, os, shlex, subprocess, threading, time, urllib.parse, urllib.request
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from xml.sax.saxutils import escape
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'test-output/drives-ui'
OUT.mkdir(parents=True, exist_ok=True)
FILES = {}; DIRS = {'', 'RustWord', 'RustWord/子目录'}; CHECKS = []; ERROR = None; TAB = None; CLOSED = False; EVENTS=[]
class DAV(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def path_name(self):
        p=urllib.parse.unquote(self.path.split('?')[0]).removeprefix('/dav').strip('/')
        EVENTS.append(self.command+' '+p); return p
    def send(self, code, data=b'', content='application/octet-stream'):
        self.send_response(code); self.send_header('Content-Length', len(data)); self.send_header('Content-Type', content); self.end_headers()
        if self.command != 'HEAD': self.wfile.write(data)
    def do_HEAD(self):
        p = self.path_name(); self.send(200 if p in DIRS or p in FILES else 404, FILES.get(p,b''))
    def do_GET(self):
        p = self.path_name(); self.send(200 if p in FILES else 404, FILES.get(p,b''))
    def do_MKCOL(self):
        p = self.path_name()
        if p in DIRS: self.send(405)
        else: DIRS.add(p); self.send(201)
    def do_PUT(self):
        p = self.path_name(); FILES[p] = self.rfile.read(int(self.headers.get('Content-Length',0))); self.send(201)
    def do_PROPFIND(self):
        p = self.path_name()
        if p not in DIRS and p not in FILES: self.send(404); return
        entries = [p]
        if p in DIRS:
            prefix = p+'/' if p else ''
            entries += sorted(x for x in DIRS|set(FILES) if x != p and x.startswith(prefix) and '/' not in x[len(prefix):])
        rows=[]
        for name in entries:
            folder=name in DIRS; href='/dav/'+urllib.parse.quote(name)+('/' if folder and name else '')
            rows.append('<d:response><d:href>'+escape(href)+'</d:href><d:propstat><d:prop><d:resourcetype>'+('<d:collection/>' if folder else '')+'</d:resourcetype><d:getcontentlength>'+str(len(FILES.get(name,b'')))+'</d:getcontentlength><d:getlastmodified>Mon, 12 Oct 2026 12:00:00 GMT</d:getlastmodified><d:getcontenttype>'+('httpd/unix-directory' if folder else 'application/octet-stream')+'</d:getcontenttype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>')
        self.send(207, ('<d:multistatus xmlns:d="DAV:">'+''.join(rows)+'</d:multistatus>').encode(), 'application/xml')
DAV_SERVER=ThreadingHTTPServer(('127.0.0.1',0),DAV)
threading.Thread(target=DAV_SERVER.serve_forever,daemon=True).start()
config=OUT/'rclone-fixture.conf'
config.write_text('[acceptance]\ntype = webdav\nurl = http://127.0.0.1:'+str(DAV_SERVER.server_port)+'/dav/\nvendor = other\n')
config.chmod(0o600)
app_root=OUT/'app-data'; app_root.mkdir(exist_ok=True)
for filename in ['drives.json','recovery.json']:(app_root/filename).unlink(missing_ok=True)
environment=dict(os.environ, RCLONE_CONFIG=str(config))
PROCESS=subprocess.Popen([str(ROOT/'target/release/rust-word'),'--test-server','0.0.0.0:18771',str(app_root)],env=environment,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
CONF={}
for line in (Path.home()/'.config/codex-chrome-mcp/env').read_text().splitlines():
    line=line.strip().removeprefix('export ')
    if line and not line.startswith('#') and '=' in line:
        k,v=line.split('=',1); tokens=shlex.split(v)
        if tokens: CONF[k]=tokens[0]
OPENER=urllib.request.build_opener(urllib.request.ProxyHandler({}))
def call(name,**arguments):
    if TAB and name!='chrome_new_tab':arguments.setdefault('tab_id',TAB)
    req=urllib.request.Request(CONF['CODEX_CHROME_API_URL_LAN']+'/api/tools/call',data=json.dumps({'name':name,'arguments':arguments}).encode(),headers={'Authorization':'Bearer '+CONF['CODEX_CHROME_API_TOKEN'],'Content-Type':'application/json'})
    raw=json.load(OPENER.open(req,timeout=45))
    if not raw.get('ok') or raw.get('result',{}).get('isError'):raise RuntimeError(str(raw))
    return json.loads(''.join(c['text'] for c in raw['result']['content'] if c['type']=='text'))
def ev(code):
    value=call('chrome_cdp',method='Runtime.evaluate',params={'expression':code,'returnByValue':True,'awaitPromise':True})
    if value.get('exceptionDetails'):raise AssertionError(value['exceptionDetails'])
    return value.get('result',{}).get('value')
def wait(code,seconds=40):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        if ev(code):return
        time.sleep(.3)
    raise AssertionError('Timeout: '+code)
def click(selector):call('chrome_click',selector=selector)
def type_text(selector,text):call('chrome_type',selector=selector,text=text,clear=True)
def check(ok,text):
    assert ok,text;CHECKS.append(text);print('PASS '+text,flush=True)
def select(selector,value):
    ev('(()=>{let e=document.querySelector('+json.dumps(selector)+');e.value='+json.dumps(value)+';e.dispatchEvent(new Event("change",{bubbles:true}));})()')
try:
    time.sleep(.8)
    TAB=call('chrome_new_tab',url='http://127.0.0.1:18771/')['tab']['id'];wait('!!window.rustWord?.drives')
    if ev("!document.querySelector('#modal').classList.contains('hidden')"):click('#modal-actions button:first-child')
    doc={'type':'doc','content':[{'type':'heading','attrs':{'level':1},'content':[{'type':'text','text':'网盘界面合成验收'}]},{'type':'paragraph','content':[{'type':'text','text':'仅含测试内容。','marks':[{'type':'bold'}]}]}]}
    ev('window.rustWord.loadDocument('+json.dumps({'name':'网盘验收','document':doc},ensure_ascii=False)+')')
    normalized_doc=ev('window.rustWord.document()')
    click('[data-tab="tools"]');click('[data-action="cloud"]');wait("document.querySelector('#drive-status').textContent.includes('1 个远端')")
    check('还没有连接' in ev("document.querySelector('#drive-connections').textContent"),'统一网盘入口显示真实空连接')
    click('#drive-add');wait("!!document.querySelector('#drive-provider')");select('#drive-provider','rclone')
    check(ev("document.querySelector('#drive-remote-path').value")== '', '切换其他网盘清除百度预设子路径')
    select('#drive-remote-select','acceptance');type_text('#drive-folder','RustWord');click('#modal-actions button:last-child')
    wait("document.querySelector('#drive-list').textContent.includes('子目录')")
    check(ev("document.querySelector('#drive-connections').textContent.includes('acceptance')"),'客户端连接经真实rclone读取后保存')
    check(not ev("document.querySelector('#drive-upload-word').disabled"),'可写连接启用上传按钮')
    click('#drive-list .drive-folder');wait("document.querySelector('#drive-path').textContent.includes('子目录')")
    check('当前文件夹为空' in ev("document.querySelector('#drive-list').textContent"),'真实子目录浏览')
    click('#drive-parent');wait("document.querySelector('#drive-list').textContent.includes('子目录')")
    check(ev("document.querySelector('#drive-path').textContent").endswith('RustWord'),'上一级回到工作目录')
    click('#drive-upload-word');wait("document.querySelector('#drive-list .drive-file')!==null || document.querySelector('#drive-status').classList.contains('error')")
    check(not ev("document.querySelector('#drive-status').classList.contains('error')"),'Word上传实际状态：'+ev("document.querySelector('#drive-status').textContent"))
    check(len(FILES)==1 and next(iter(FILES.values())).startswith(b'PK'),'当前Word按钮通过rclone真实PUT可编辑DOCX')
    file_data=next(iter(FILES.values())); original_sha=hashlib.sha256(file_data).hexdigest()
    saved=json.loads((app_root/'drives.json').read_text());existing_name=next(iter(FILES)).split('/')[-1]
    duplicate=ev('window.rustWord.invoke("driveUpload",'+json.dumps({'connectionId':saved[0]['id'],'path':'','name':existing_name,'data':base64.b64encode(b'replacement').decode()})+').then(()=>"incorrect success",e=>e.message)')
    check('同名' in duplicate and hashlib.sha256(next(iter(FILES.values()))).hexdigest()==original_sha,'重复上传拒绝覆盖并保留远端原件')
    click('#drive-list .drive-file button:last-child');wait("document.querySelector('#drive-status').textContent.includes('本机副本')")
    check(ev('window.rustWord.document()')==normalized_doc,'远端DOCX下载并解析保留标题粗体')
    check('未保存' in ev("document.querySelector('#save-state').textContent"),'打开副本要求另存文件')
    click('#drive-refresh');wait("!document.querySelector('#drive-overlay').classList.contains('busy')")
    (OUT/'drives-connected.png').write_bytes(base64.b64decode(call('chrome_cdp',method='Page.captureScreenshot',params={'format':'png'})['data']))
    click('#drive-remove');wait("document.querySelector('#modal-title').textContent.includes('移除')");click('#modal-actions button:last-child');wait("document.querySelector('#drive-connections').textContent.includes('还没有连接')")
    check(len(FILES)==1,'移除连接不删除远端文件')
    click('#drive-add');wait("!!document.querySelector('#drive-provider')");select('#drive-provider','rclone');select('#drive-remote-select','acceptance');type_text('#drive-folder','RustWord')
    ev("document.querySelector('#drive-readonly').checked=true;document.querySelector('#drive-create').checked=false")
    click('#modal-actions button:last-child');wait("document.querySelector('#drive-list .drive-file')!==null")
    check(ev("document.querySelector('#drive-upload-word').disabled && document.querySelector('#drive-upload-file').disabled"),'只读连接禁用两种上传按钮')
    saved=json.loads((app_root/'drives.json').read_text());check(all(set(r)=={'id','provider','remote','folder','readonly'} for r in saved),'连接持久化仅保存元数据不复制密码')
    check((app_root/'drives.json').stat().st_mode&0o777==0o600,'连接配置仅本人可读写')
    click('#drive-webdav');wait("document.querySelector('#suite-title').textContent==='个人云空间'")
    check(ev("document.querySelector('#drive-overlay').classList.contains('hidden')"),'从网盘Hub进入已有iCloud/文件夹/WebDAV面板')
    click('#suite-close');ev("window.rustWord.invoke('discardRecovery')")
except Exception as error:
    ERROR=str(error);print('FAIL '+ERROR,flush=True)
    if TAB:
        try:print('UI STATUS '+ev("document.querySelector('#drive-status').textContent"),flush=True)
        except Exception:pass
    fixture_file=OUT/'diagnostic.txt';fixture_file.write_text('synthetic diagnostic')
    for arguments in [['lsjson','--stat','acceptance:RustWord/missing.docx'],['copyto','--ignore-existing','--immutable','--no-traverse',str(fixture_file),'acceptance:RustWord/diagnostic.docx']]:
        debug=subprocess.run(['/opt/homebrew/bin/rclone',*arguments,'--log-level','ERROR','--retries','1','--low-level-retries','1'],env=environment,capture_output=True,text=True,timeout=15)
        print('FIXTURE ONLY '+str(debug.returncode)+' '+debug.stderr.strip(),flush=True)
finally:
    if TAB:
        try:call('chrome_close_tab');CLOSED=True
        except Exception as error:ERROR=ERROR or str(error)
    PROCESS.terminate();PROCESS.wait(timeout=8);DAV_SERVER.shutdown()
    (OUT/'checks.json').write_text(json.dumps({'passed':len(CHECKS),'checks':CHECKS,'error':ERROR,'ownedTabClosed':CLOSED,'fixture':'isolated real WebDAV + rclone, not a live provider claim','protocolEvents':EVENTS,'fixtureFiles':{k:len(v) for k,v in FILES.items()}},ensure_ascii=False,indent=2)+'\n')
if ERROR:raise SystemExit(1)
