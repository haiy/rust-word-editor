import {test} from 'node:test';
import assert from 'node:assert/strict';
import {runInNewContext} from 'node:vm';
import {readFileSync} from 'node:fs';
import {processJs,translate} from '../scripts/localization.mjs';
const catalog=JSON.parse(readFileSync(new URL('../locales/en.json',import.meta.url),'utf8'));
test('compiled UI translates labels without rewriting dynamic user text',()=>{
 const code=processJs("const user='user supplied'; result=[`已保存 ${user}`, '保存', ({'宋体':'Songti SC'})['\\u5b8b\\u4f53']];",catalog);
 const context={};runInNewContext(code,context);
 assert.match(context.result[0],/user supplied/);assert.equal(context.result[1],'Save');assert.equal(context.result[2],'Songti SC');
});
test('document text passed at runtime survives English compilation unchanged',()=>{
 const code=processJs('result = {label:"保存",document:documentText};',catalog);
 const c={documentText:'用户原文 Save 中文 English'};runInNewContext(code,c);assert.equal(c.result.document,c.documentText);assert.equal(c.result.label,'Save');
});
test('translated quotes and template metacharacters stay literal',()=>{
 const code=processJs('result = `保存 ${user}`;',{'保存':"Save ` ${notCode} ' \\"});
 const c={user:'中文'};runInNewContext(code,c);assert.equal(c.result,"Save ` ${notCode} ' \\ 中文");
});
test('incomplete catalogs fail closed',()=>{assert.throws(()=>translate('未翻译的功能',{}),/Missing English translation/);});
test('HTML category values and matching code literals agree',()=>{
 const html=translate('<button data-category="工作">工作</button>',catalog);
 const code=processJs("result='工作'",catalog);const c={};runInNewContext(code,c);assert.ok(html.includes(`data-category="${c.result}"`));
});
