#!/usr/bin/env python3
"""Regenerate original MIT-licensed starter decks; requires python-pptx.
No third-party presentation designs, images or proprietary templates are used.
"""
from pathlib import Path
from io import BytesIO
import json,hashlib,zipfile
from pptx import Presentation
from pptx.util import Inches,Pt
from pptx.dml.color import RGBColor
from pptx.enum.shapes import MSO_SHAPE
ROOT=Path(__file__).resolve().parents[1]
THEMES=[('blue','Clear Blue','清晰蓝','235BAB','D7E4F4'),('green','Forest','森林绿','26735C','D8EFE4'),('purple','Violet','紫罗兰','6847A6','E7DEF5')]
TOPICS=[
 ('work','Work Update','工作汇报',['Results','Progress','Next Steps'],['本期成果','项目进度','下一步计划']),
 ('project','Project Plan','项目计划',['Goals','Milestones','Risks'],['项目目标','里程碑','风险应对']),
 ('analysis','Analysis Report','分析报告',['Findings','Evidence','Recommendations'],['核心发现','事实依据','行动建议']),
 ('training','Training','培训课程',['Objectives','Practice','Review'],['学习目标','实践任务','总结回顾']),
 ('product','Product Overview','产品介绍',['User Needs','Solution','Getting Started'],['用户需求','产品方案','开始使用']),
 ('simple','Simple Presentation','简约演示',['Background','Main Points','Next Steps'],['背景说明','主要内容','下一步'])]
parts={};rows=[];themes=[];theme_assets=[]
def package(entries):
 out=BytesIO()
 with zipfile.ZipFile(out,'w',zipfile.ZIP_DEFLATED) as z:
  for name,data in sorted(entries.items()):
   info=zipfile.ZipInfo(name,(2026,1,1,0,0,0));info.compress_type=zipfile.ZIP_DEFLATED;z.writestr(info,data)
 return out.getvalue()
def text(slide,value,x,y,w,h,size,color,bold=False):
 box=slide.shapes.add_textbox(Inches(x), Inches(y), Inches(w), Inches(h));tf=box.text_frame;tf.word_wrap=True
 for i,line in enumerate(value.split('\n')):
  p=tf.paragraphs[0] if i==0 else tf.add_paragraph();p.text=line;p.font.size=Pt(size);p.font.bold=bold;p.font.name='Arial';p.font.color.rgb=RGBColor.from_string(color)
  p.space_after=Pt(16)
for ix,(key,en,zh,accent,tint) in enumerate(THEMES):
 themes.append({'id':'public-'+key,'name':en,'nameZh':zh,'colors':{'accent1':accent,'accent2':tint,'accent3':'E3A34F','accent4':'668AAE','accent5':'7867A4','accent6':'AD765A','dk1':'172B42','dk2':'344D67','lt1':'FFFFFF','lt2':'F3F6FB','hlink':'235BAB','folHlink':'6847A6'},'fonts':{'major':'Arial','minor':'Arial','majorEastAsia':'PingFang SC','minorEastAsia':'PingFang SC'}})
for n,(key,en,zh,en_pages,zh_pages) in enumerate(TOPICS):
 theme=themes[n%3];accent=theme['colors']['accent1']
 for locale,title,headings in [('en',en,en_pages),('zh-CN',zh,zh_pages)]:
  prs=Presentation();prs.slide_width=Inches(13.333333);prs.slide_height=Inches(7.5)
  prs.core_properties.author='Rust Word contributors';prs.core_properties.title=title;prs.core_properties.subject='Original starter template';prs.core_properties.keywords='MIT, Rust Word'
  for i,heading in enumerate([title]+headings):
   slide=prs.slides.add_slide(prs.slide_layouts[6]);bg=accent if i==0 else 'FFFFFF'
   slide.background.fill.solid();slide.background.fill.fore_color.rgb=RGBColor.from_string(bg)
   bar=slide.shapes.add_shape(MSO_SHAPE.RECTANGLE,0,0,Inches(.16),prs.slide_height);bar.fill.solid();bar.fill.fore_color.rgb=RGBColor.from_string(theme['colors']['accent2']);bar.line.fill.background()
   text(slide,heading,.8,2.0 if i==0 else .65,11.5,1.3,36,'FFFFFF' if i==0 else accent,True)
   body=('Your name · Team · Date' if locale=='en' else '姓名 · 团队 · 日期') if i==0 else ('Add a clear point with supporting evidence.\nDescribe an action, owner and deadline.' if locale=='en' else '填写一个明确观点，并补充事实依据。\n说明具体行动、负责人和完成时间。')
   text(slide,body,.85,3.8 if i==0 else 2.3,11.0,2.6,22,'FFFFFF' if i==0 else '172B42')
   text(slide,f'Rust Word · {i+1:02d}',.85,6.9,10,.35,10,'FFFFFF' if i==0 else '344D67')
  output=BytesIO();prs.save(output)
  with zipfile.ZipFile(BytesIO(output.getvalue())) as z: deck={name:z.read(name) for name in z.namelist()}
  # Strip optional preview and deterministic timestamps from Office metadata.
  import re
  for name in ['docProps/core.xml']:
   deck[name]=re.sub(rb'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z',b'2026-01-01T00:00:00Z',deck[name])
  data=package(deck);entry=f'original/{key}-{locale}.pptx';parts[entry]=data;sha=hashlib.sha256(data).hexdigest()
  rows.append({'id':f'public-{key}-{locale}','name':title,'language':locale,'native':True,'slideCount':4,'conversion':'original-ooxml','nativeEntry':entry,'originalEntry':entry,'originalName':f'{key}-{locale}.pptx','nativeSha256':sha,'sourceSha256':sha,'license':'MIT'})
# Theme download packages contain newly selected palettes and standard Office schema.
base=next(iter(parts.values()))
with zipfile.ZipFile(BytesIO(base)) as z: raw_theme=z.read('ppt/theme/theme1.xml').decode()
for t in themes:
 theme=raw_theme
 for name,color in t['colors'].items():
  theme=re.sub(rf'<a:{name}>.*?</a:{name}>',f'<a:{name}><a:srgbClr val="{color}"/></a:{name}>',theme)
 theme=re.sub(r'name="Office Theme"',f'name="{t["name"]}"',theme)
 data=package({'[Content_Types].xml':'<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/theme/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>','_rels/.rels':'<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="theme/theme/theme1.xml"/></Relationships>','theme/theme/theme1.xml':theme})
 entry=f'themes/{t["id"]}.thmx';parts[entry]=data
 theme_assets.append({'id':t['id'],'name':t['name'],'originalName':t['id']+'.thmx','originalEntry':entry,'sourceSha256':hashlib.sha256(data).hexdigest(),'native':False,'license':'MIT'})
(ROOT/'src/ppt-template-pack.zip').write_bytes(package(parts))
(ROOT/'src/ppt-theme-data.json').write_text(json.dumps({'themes':themes,'license':'MIT','source':'Original Rust Word palettes'},ensure_ascii=False,indent=2)+'\n')
(ROOT/'src/ppt-template-data.json').write_text(json.dumps({'templates':rows,'themeAssets':theme_assets,'license':'MIT','source':'Original Rust Word starter content'},ensure_ascii=False,indent=2)+'\n')
print(len(rows),'original decks;',len(themes),'themes;', (ROOT/'src/ppt-template-pack.zip').stat().st_size,'bytes')
