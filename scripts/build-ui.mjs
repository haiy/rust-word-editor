import { build } from 'esbuild';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { processJs, translate } from './localization.mjs';
const catalog=JSON.parse(await readFile('locales/en.json','utf8'));
const template=await readFile('ui/index.html','utf8');
const sourceBody=template.match(/<body>([\s\S]*?)<script>/)[1];
const css=(await Promise.all(['style','features','suite','ppt','drives'].map(name=>readFile(`ui/${name}.css`,'utf8')))).join('\n');
const languages={};
for(const locale of ['en','zh-CN']){
 const result=await build({entryPoints:['ui/editor.js'],bundle:true,write:false,minify:true,format:'iife',target:'safari15',plugins:locale==='en'?[{name:'english-ui',setup(b){b.onLoad({filter:/ui\/.*\.js$/},async args=>({contents:processJs(await readFile(args.path,'utf8'),catalog),loader:'js'}));}}]:[]});
 languages[locale]={body:locale==='en'?translate(sourceBody,catalog):sourceBody,script:result.outputFiles[0].text};
}
const boot=`(async()=>{
 const languages=${JSON.stringify(languages).replaceAll('</script','<\\/script')};
 const req={id:-9001,command:'uiPreferences',browserLanguage:navigator.language};
 try{
 const reply=window.ipc?await new Promise((resolve,reject)=>{window.__rustReply=r=>{if(r.id===req.id)r.error?reject(new Error(r.error)):resolve(r);};window.ipc.postMessage(JSON.stringify(req));}):await fetch('/rpc',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(req)}).then(r=>r.json());
 if(reply.error)throw new Error(reply.error);
 const locale=reply.data.locale==='zh-CN'?'zh-CN':'en';window.__rustWordBoot={...reply.data,locale};
 document.documentElement.lang=locale;document.body.innerHTML=languages[locale].body;
 const script=document.createElement('script');script.textContent=languages[locale].script;document.body.append(script);
 }catch(e){document.body.textContent='Unable to start Rust Word / 无法启动：'+e.message;}
})();`;
await mkdir('dist',{recursive:true});
await writeFile('dist/index.html',`<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Rust Word</title><style>${css}</style></head><body><p>Loading Rust Word…</p><script>${boot.replaceAll('</script','<\\/script')}</script></body></html>`);
console.log('Bundled offline English and Chinese interfaces: dist/index.html');
