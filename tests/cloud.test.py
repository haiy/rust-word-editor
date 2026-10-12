"""Exercise the shipped Rust WebDAV adapter against a real local protocol server."""
from pathlib import Path
import base64,json,os,subprocess,threading,urllib.parse,zipfile
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'test-output/cloud';OUT.mkdir(parents=True,exist_ok=True)
FILES={};events=[]
class DAV(BaseHTTPRequestHandler):
 def log_message(self,*args): pass
 def auth(self):
  if self.headers.get('Authorization')!='Basic '+base64.b64encode(b'fixture-user:fixture-password').decode():
   self.send_response(401);self.end_headers();return False
  return True
 def do_PROPFIND(self):
  if not self.auth():return
  entries=['<d:response><d:href>/dav/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>']
  for name,body in FILES.items():entries.append(f'<d:response><d:href>/dav/{urllib.parse.quote(name)}</d:href><d:propstat><d:prop><d:displayname>{name}</d:displayname><d:getcontentlength>{len(body)}</d:getcontentlength><d:resourcetype/></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>')
  body=('<d:multistatus xmlns:d="DAV:">'+''.join(entries)+'</d:multistatus>').encode();self.send_response(207);self.send_header('Content-Type','application/xml');self.send_header('Content-Length',len(body));self.end_headers();self.wfile.write(body);events.append('PROPFIND Depth='+self.headers.get('Depth',''))
 def do_PUT(self):
  if not self.auth():return
  name=urllib.parse.unquote(self.path.removeprefix('/dav/'))
  if name in FILES and self.headers.get('If-None-Match')=='*':self.send_response(412);self.end_headers();return
  body=self.rfile.read(int(self.headers.get('Content-Length',0)));FILES[name]=body;self.send_response(201);self.end_headers();events.append('PUT '+name)
 def do_GET(self):
  if not self.auth():return
  name=urllib.parse.unquote(self.path.removeprefix('/dav/'));body=FILES.get(name)
  self.send_response(200 if body is not None else 404);self.send_header('Content-Length',len(body or b''));self.end_headers();self.wfile.write(body or b'');events.append('GET '+name)
server=ThreadingHTTPServer(('127.0.0.1',0),DAV);threading.Thread(target=server.serve_forever,daemon=True).start()
proc=subprocess.Popen([str(ROOT/'target/debug/rust-word'),'--rpc',str(OUT/'rpc')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
checks=[]
def rpc(command,**kw):
 proc.stdin.write(json.dumps({'id':len(checks)+1,'command':command,**kw})+'\n');proc.stdin.flush();v=json.loads(proc.stdout.readline());return v

def call(command,**kw):
 v=rpc(command,**kw)
 if 'error' in v:raise AssertionError(v['error'])
 return v['data']
def check(value,message):
 assert value,message;checks.append(message);print('PASS '+message,flush=True)
try:
 endpoint=f'http://127.0.0.1:{server.server_port}/dav/'
 current=call('saveCloudSettings',settings={'endpoint':endpoint,'username':'fixture-user','password':'fixture-password'})
 check(current['provider']=='webdav' and current['hasPassword'] and 'password' not in current,'配置保存并隐藏密码')
 check(not call('cloudList')['files'],'真实PROPFIND空目录')
 doc={'type':'doc','content':[{'type':'paragraph','content':[{'type':'text','text':'云文件验收正文'}]}]}
 export=call('exportDocument',document=doc,name='云文件验收');export['name']='中文 云文档.docx'
 call('cloudUpload',**export)
 listed=call('cloudList')['files'];check(listed[0]['name']==export['name'] and listed[0]['bytes']==export['bytes'],'PUT真实文件与UTF-8名称，PROPFIND正确解析')
 downloaded=call('cloudDownload',name=export['name']);check(downloaded['data']==export['data'],'GET内容字节一致')
 opened=call('importDocument',**downloaded);check(opened['document']==doc,'下载的DOCX真实解析为可编辑原模型')
 check('同名' in rpc('cloudUpload',**export)['error'],'If-None-Match阻止覆盖现有云文件')
 check('error' in rpc('cloudDownload',name='../outside.docx'),'拒绝越界云文件名')
 call('saveCloudSettings',settings={'endpoint':endpoint,'username':'fixture-user','password':'wrong'})
 check('HTTP 401' in rpc('cloudTest')['error'],'真实认证失败不会显示成功')
 call('saveCloudSettings',settings={'endpoint':endpoint,'username':'fixture-user','password':'fixture-password'})
 check('远程 WebDAV' in call('cloudTest')['status'],'认证修正后真实重连')
 folder=OUT/'rpc/synced';folder.mkdir(exist_ok=True)
 call('cloudSetFolderPath',path=str(folder));call('cloudUpload',**export)
 check((folder/export['name']).read_bytes()==base64.b64decode(export['data']),'同步文件夹落盘DOCX与导出一致')
 check(call('cloudDownload',name=export['name'])['data']==export['data'],'同步文件夹重新读回')
 check(os.stat(OUT/'rpc/cloud.json').st_mode&0o777==0o600,'本机配置仅本人可读写')
 (OUT/'checks.json').write_text(json.dumps({'passed':len(checks),'checks':checks,'protocolEvents':events},ensure_ascii=False,indent=2)+'\n')
finally:
 proc.terminate();proc.wait(timeout=5);server.shutdown()
