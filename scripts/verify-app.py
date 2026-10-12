"""Validate a signed bundle's executable sections, helper and actual RPC version."""
from pathlib import Path
import hashlib,json,struct,subprocess,tempfile,base64,zlib,tomllib
ROOT=Path(__file__).resolve().parents[1]
APP=ROOT/'build/Rust Word.app'
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def sections(path):
 data=path.read_bytes();assert struct.unpack_from('<I',data)[0]==0xfeedfacf
 count=struct.unpack_from('<I',data,16)[0];offset=32;result={}
 for _ in range(count):
  cmd,size=struct.unpack_from('<II',data,offset)
  if cmd==0x19:
   n=struct.unpack_from('<I',data,offset+64)[0]
   for i in range(n):
    s=offset+72+i*80;name=data[s:s+16].split(b'\0')[0].decode();segment=data[s+16:s+32].split(b'\0')[0].decode();length=struct.unpack_from('<Q',data,s+40)[0];fileoff=struct.unpack_from('<I',data,s+48)[0];flags=struct.unpack_from('<I',data,s+64)[0]&0xff
    if fileoff and length and flags not in [1,12,18]:result[segment+'/'+name]=hashlib.sha256(data[fileoff:fileoff+length]).hexdigest()
  offset+=size
 return result
built=ROOT/'target/release/rust-word';signed=APP/'Contents/MacOS/rust-word';helper=APP/'Contents/MacOS/rust-word-media'
assert sections(built)==sections(signed),'Signed executable differs from validated build'
subprocess.run(['codesign','--verify','--deep','--strict',str(APP)],check=True)
with tempfile.TemporaryDirectory(dir=ROOT/'build') as tmp:
 result=subprocess.run([str(signed),'--rpc',tmp],input=json.dumps({'id':1,'command':'ready'})+'\n',text=True,capture_output=True,check=True)
 version=json.loads(result.stdout)['data']['version'];assert version==tomllib.loads((ROOT/'Cargo.toml').read_text())['package']['version']
 def chunk(kind,payload):return struct.pack('>I',len(payload))+kind+payload+struct.pack('>I',zlib.crc32(kind+payload)&0xffffffff)
 png=base64.b64encode(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(b'\0\xff\xff\xff\xff'))+chunk(b'IEND',b'')).decode()

 result=subprocess.run([str(signed),'--rpc',tmp],input=json.dumps({'id':2,'command':'imagesToPdf','images':['data:image/png;base64,'+png]})+'\n',text=True,capture_output=True,check=True)
 response=json.loads(result.stdout);assert 'data' in response and response['data']['pageCount']==1,response
 def rpc(command,**args):
  result=subprocess.run([str(signed),'--rpc',tmp],input=json.dumps({'id':3,'command':command,**args},ensure_ascii=False)+'\n',text=True,capture_output=True,check=True,timeout=120)
  response=json.loads(result.stdout);assert 'data' in response,response
  return response['data']
 assert (APP/'Contents/Resources/LibreOfficeDev.app/Contents/MacOS/soffice').is_file()
 assert (APP/'Contents/Resources/rclone-LICENSE.txt').is_file()
 discovery=rpc('driveDiscover');assert discovery['available'] and 'rclone' in discovery['version']
 deck=rpc('pptTemplateOpen',templateId='public-work-en')
 assert deck['native'] and deck['slides'][0]['textSlots']
 deck['slides'][0]['textSlots'][0]['text']='新版应用中文渲染验收'
 rendered=rpc('pptNativeRender',deck=deck,pages=[0])
 assert len(rendered['images'])==1 and rendered['images'][0]['data'].startswith('data:image/png;base64,')
 out=ROOT/'test-output/bundled-ppt-preview.png';out.parent.mkdir(exist_ok=True)
 out.write_bytes(base64.b64decode(rendered['images'][0]['data'].split(',',1)[1]))
record={'version':version,'signedBinarySha256':sha(signed),'cargoBinarySha256':sha(built),'helperSha256':sha(helper),'codeSectionsMatch':True,'signatureVerified':True,'bundledPdfHelperRpcPassed':True,'bundledRclonePassed':True,'bundledNativePptRenderPassed':True,'nativePreviewSha256':sha(out)}
(ROOT/'test-output/app-verification.json').write_text(json.dumps(record,ensure_ascii=False,indent=2)+'\n');print(json.dumps(record,ensure_ascii=False))
