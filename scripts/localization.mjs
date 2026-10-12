import {parse} from '@babel/parser';
import traverseModule from '@babel/traverse';
import generateModule from '@babel/generator';
import {readFileSync,readdirSync,writeFileSync} from 'node:fs';
const traverse=traverseModule.default || traverseModule;
const generate=generateModule.default || generateModule;
export const hasChinese=s=>/\p{Script=Han}/u.test(s);
export function pieces(value){return value.split(/([<>{}"'\n`]+)/).filter(s=>hasChinese(s)).map(s=>s.trim()).filter(Boolean);}
export function translate(value,catalog,strict=true){
 return value.split(/([<>{}"'\n`]+)/).map(part=>{
  if(!hasChinese(part))return part;
  const key=part.trim(), translated=catalog[key];
  if(translated===undefined){if(strict)throw new Error('Missing English translation: '+key);return part;}
  return part.slice(0,part.indexOf(key))+translated+part.slice(part.indexOf(key)+key.length);
 }).join('');
}
export function processJs(source,catalog,collect){
 const ast=parse(source,{sourceType:'module'});
 const visit=value=>{if(collect)pieces(value).forEach(s=>collect.add(s));return catalog?translate(value,catalog):value;};
 traverse(ast,{
  StringLiteral(p){if((p.parent.type==="ObjectProperty" && p.key==="key") || (p.parent.type==="MemberExpression" && p.key==="property"))return;p.node.value=visit(p.node.value); if(catalog)delete p.node.extra;},
  TemplateElement(p){const n=p.node; if(collect)visit(n.value.cooked??n.value.raw);if(catalog){const text=visit(n.value.cooked??n.value.raw);n.value={raw:text.replace(/\\/g,'\\\\').replace(/`/g,'\\`').replace(/\$\{/g,'\\${'),cooked:text};}}
 });
 return catalog?generate(ast).code:source;
}
if(process.argv.includes('--extract')){
 const set=new Set();
 for(const f of readdirSync('ui').filter(f=>f.endsWith('.js')))processJs(readFileSync('ui/'+f,'utf8'),null,set);
 pieces(readFileSync('ui/index.html','utf8')).forEach(s=>set.add(s));
 // Backend service messages are translated at the presentation boundary, never in document contents.
 for(const dir of ['src','scripts'])for(const f of readdirSync(dir).filter(f=>f.endsWith('.rs')||f.endsWith('.swift'))){
  const source=readFileSync(dir+'/'+f,'utf8').split('#[cfg(test)]')[0];
  for(const m of source.matchAll(/"((?:\\.|[^"\\])*)"/g)){
   let value=m[1];try{value=JSON.parse('"'+value+'"');}catch{}
   pieces(value).forEach(s=>set.add(s));
  }
 }
 const keys=[...set].sort();writeFileSync('localization-work/keys.json',JSON.stringify(keys,null,2));
 console.log(keys.length+' strings; '+keys.join('').length+' characters');
}
