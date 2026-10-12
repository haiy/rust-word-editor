"""Public starter decks: archive, model, text editing and theme download checks."""
import base64,hashlib,json,os,subprocess,tempfile,zipfile,io
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
BINARY=os.environ.get('RUST_WORD_TEST_BINARY',str(ROOT/'target/release/rust-word'))
with tempfile.TemporaryDirectory() as tmp:
 def rpc(command,**data):
  r=subprocess.run([BINARY,'--rpc',tmp],input=json.dumps({'id':1,'command':command,**data})+'\n',text=True,capture_output=True,check=True)
  value=json.loads(r.stdout);assert 'error' not in value,value
  return value['data']
 catalog=rpc('pptThemes');rows=catalog['templateCatalog']['templates'];assert len(rows)==12
 assert {r['language'] for r in rows}=={'en','zh-CN'}
 for row in rows:
  file=rpc('pptTemplateFile',templateId=row['id']);raw=base64.b64decode(file['data'])
  assert hashlib.sha256(raw).hexdigest()==row['nativeSha256']
  source={n:zipfile.ZipFile(io.BytesIO(raw)).read(n) for n in zipfile.ZipFile(io.BytesIO(raw)).namelist()}
  deck=rpc('pptTemplateOpen',templateId=row['id']);assert deck['native'] and len(deck['slides'])==4
  deck['slides'][0]['textSlots'][0]['text']='Public edit 中文 English'
  out=rpc('pptExport',deck=deck);pack=zipfile.ZipFile(io.BytesIO(base64.b64decode(out['data'])))
  assert any(b'Public edit' in pack.read(n) for n in pack.namelist() if n.startswith('ppt/slides/') and n.endswith('.xml'))
  for n in source:
   if n.startswith(('ppt/slideMasters/','ppt/slideLayouts/','ppt/theme/','ppt/media/')):assert source[n]==pack.read(n),(row['id'],n)
 for row in catalog['templateCatalog']['themeAssets']:
  file=rpc('pptTemplateFile',templateId=row['id'],original=True);raw=base64.b64decode(file['data'])
  assert hashlib.sha256(raw).hexdigest()==row['sourceSha256']
  assert 'theme/theme/theme1.xml' in zipfile.ZipFile(io.BytesIO(raw)).namelist()
 assert len(catalog['themes'])==3
 print('PASS: 12 editable bilingual decks, preserved artwork, 3 downloadable themes')
