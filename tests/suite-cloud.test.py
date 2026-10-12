"""Real cloud controls in an owned persistent Chrome tab with isolated Rust state."""
import base64,json,os,shlex,threading,time,urllib.request,urllib.parse
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'test-output/suite-cloud';OUT.mkdir(parents=True,exist_ok=True)
URL=os.environ.get('WORD_TEST_URL','http://127.0.0.1:18770/');CONF={}
for line in (Path.home()/'.config/codex-chrome-mcp/env').read_text().splitlines():
 line=line.strip().removeprefix('export ')
 if line and not line.startswith('#') and '=' in line:
  k,v=line.split('=',1);tokens=shlex.split(v)
  if tokens:CONF[k]=tokens[0]
OPENER=urllib.request.build_opener(urllib.request.ProxyHandler({}));TAB=None;CHECKS=[];ERROR=None;FILES={}
class DAV(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def auth(self):
  if self.headers.get('Authorization')!='Basic '+base64.b64encode(b'ui-user:ui-password').decode():self.send_response(401);self.end_headers();return False
  return True
 def do_PROPFIND(self):
  if not self.auth():return
  rows=['<d:response><d:href>/dav/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>']
  for name,data in FILES.items():rows.append('<d:response><d:href>/dav/'+urllib.parse.quote(name)+'</d:href><d:propstat><d:prop><d:getcontentlength>'+str(len(data))+'</d:getcontentlength><d:resourcetype/></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>')
  body=('<d:multistatus xmlns:d="DAV:">'+''.join(rows)+'</d:multistatus>').encode();self.send_response(207);self.send_header('Content-Type','application/xml');self.end_headers();self.wfile.write(body)
 def do_PUT(self):
  if not self.auth():return
  name=urllib.parse.unquote(self.path.removeprefix('/dav/'))
  if name in FILES:self.send_response(412);self.end_headers();return
  FILES[name]=self.rfile.read(int(self.headers.get('Content-Length',0)));self.send_response(201);self.end_headers()
 def do_GET(self):
  if not self.auth():return
  name=urllib.parse.unquote(self.path.removeprefix('/dav/'));data=FILES.get(name);self.send_response(200 if data else 404);self.end_headers();self.wfile.write(data or b'')
server=ThreadingHTTPServer(('127.0.0.1',0),DAV);threading.Thread(target=server.serve_forever,daemon=True).start()
def call(name,**arguments):
 if TAB and name!='chrome_new_tab':arguments.setdefault('tab_id',TAB)
 req=urllib.request.Request(CONF['CODEX_CHROME_API_URL_LAN']+'/api/tools/call',data=json.dumps({'name':name,'arguments':arguments}).encode(),headers={'Authorization':'Bearer '+CONF['CODEX_CHROME_API_TOKEN'],'Content-Type':'application/json'})
 raw=json.load(OPENER.open(req,timeout=50))
 if not raw.get('ok') or raw.get('result',{}).get('isError'):raise RuntimeError(str(raw))
 texts=[c['text'] for c in raw.get('result',{}).get('content',[]) if c.get('type')=='text']
 return json.loads(texts[0]) if texts else raw
def evaluate(code):
 r=call('chrome_cdp',method='Runtime.evaluate',params={'expression':code,'awaitPromise':True,'returnByValue':True})
 if 'exceptionDetails' in r:raise RuntimeError(str(r['exceptionDetails']))
 return r.get('result',{}).get('value')
def wait_for(code,seconds=12):
 end=time.time()+seconds
 while time.time()<end:
  if evaluate(code):return
  time.sleep(.35)
 raise AssertionError('Timeout '+code)
def click(selector):call('chrome_click',selector=selector)
def type_text(selector,value):call('chrome_type',selector=selector,text=value,clear=True)
def check(condition,label):assert condition,label;CHECKS.append(label);print('PASS '+label,flush=True)
def screenshot(name):(OUT/name).write_bytes(base64.b64decode(call('chrome_cdp',method='Page.captureScreenshot',params={'format':'png'})['data']))
try:
 (ROOT/'test-output/server-18770/cloud.json').unlink(missing_ok=True)
 call('chrome_status');TAB=call('chrome_new_tab',url=URL)['tab']['id'];wait_for('!!window.rustWord?.suite')
 if evaluate("!document.querySelector('#modal').classList.contains('hidden')"):click('#modal-actions button:first-child')
 doc={'type':'doc','content':[{'type':'heading','attrs':{'level':1},'content':[{'type':'text','text':'云空间验收'}]},{'type':'paragraph','content':[{'type':'text','text':'这份文件只包含合成测试内容。','marks':[{'type':'bold'}]}]}]}
 evaluate('window.rustWord.loadDocument('+json.dumps({'name':'云空间验收','document':doc},ensure_ascii=False)+')')
 click('[data-tab="tools"]');check(evaluate("document.querySelector('[data-panel=tools]').getBoundingClientRect().height")<=80,'新工具页保留80像素紧凑工具栏')
 screenshot('suite-tools.png');click('[data-action="cloud"]');wait_for("document.querySelector('#suite-title').textContent==='个人云空间'")
 check(evaluate("document.querySelector('#cloud-connection').textContent")=='尚未连接','未配置云空间有真实状态')
 click('#cloud-refresh');wait_for("document.querySelector('#suite-status').classList.contains('error')")
 check('请先连接' in evaluate("document.querySelector('#suite-status').textContent"),'未连接刷新明确失败，不显示假云文件')
 type_text('#cloud-endpoint',f'http://127.0.0.1:{server.server_port}/dav/');type_text('#cloud-user','ui-user');type_text('#cloud-password','ui-password');click('#cloud-save-settings');wait_for("document.querySelector('#suite-status').textContent.includes('已配置')")
 check(evaluate("document.querySelector('#cloud-password').value")== '','保存后密码输入清空')
 click('#cloud-refresh');wait_for("document.querySelector('#suite-status').textContent.includes('远程 WebDAV')")
 check(evaluate("document.querySelector('#cloud-files').textContent.includes('还没有')"),'真实WebDAV空目录显示')
 click('#cloud-upload-word');wait_for("!!document.querySelector('#cloud-files .suite-file')")
 check(len(FILES)==1 and next(iter(FILES.values())).startswith(b'PK'),'上传当前Word按钮生成实际远端DOCX')
 check(evaluate("document.querySelector('.suite-file').textContent.includes('云空间验收')"),'上传后文件列表刷新并显示新文件')
 screenshot('suite-cloud-connected.png')
 before=evaluate('window.rustWord.document()');click('.suite-file button:last-child');wait_for("document.querySelector('#suite-status').textContent.includes('作为副本')")
 check(evaluate('window.rustWord.document()')==before,'云文档打开副本保留正文、标题、粗体')
 check(evaluate("document.querySelector('#save-state').textContent.includes('未保存')"),'打开云文档副本要求另存')
 click('#suite-close');check(evaluate("document.querySelector('#suite-overlay').classList.contains('hidden')"),'关闭云面板返回Word')
 evaluate("window.rustWord.invoke('discardRecovery')")
except Exception as e:
 ERROR=str(e);print('FAIL '+ERROR,flush=True)
 if TAB:
  try:screenshot('failure.png')
  except Exception:pass
finally:
 if TAB:
  try:call('chrome_close_tab')
  except Exception as e:ERROR=ERROR or str(e)
 server.shutdown();(OUT/'checks.json').write_text(json.dumps({'url':URL,'passed':len(CHECKS),'checks':CHECKS,'error':ERROR,'ownedTabClosed':bool(TAB)},ensure_ascii=False,indent=2)+'\n')
if ERROR:raise SystemExit(1)
