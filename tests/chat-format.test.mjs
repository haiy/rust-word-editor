import {test} from 'node:test';
import assert from 'node:assert/strict';
import {answerBlocks} from '../ui/chat.js';
test('chat responses become editable heading, list and code nodes',()=>{
 const blocks=answerBlocks('# Title\n\nA **bold** point.\n\n- One\n- Two\n\n```text\n<example>\n```');
 assert.deepEqual(blocks.map(b=>b.type),['heading','paragraph','bulletList','codeBlock']);
 assert.equal(blocks[1].content[1].marks[0].type,'bold');
 assert.equal(blocks[2].content.length,2);
 assert.equal(blocks[3].content[0].text,'<example>');
});
test('model HTML and script-like strings remain literal text',()=>{
 const raw='<img src=x onerror=alert(1)>\n<script>bad()</script>';
 assert.deepEqual(answerBlocks(raw),[{type:'paragraph',content:[{type:'text',text:raw}]}]);
});
test('ordinary text, Chinese and unfinished code fences are retained',()=>{
 assert.equal(answerBlocks('中文\nNext line')[0].content[0].text,'中文\nNext line');
 assert.equal(answerBlocks('```\nunfinished')[0].content[0].text,'unfinished');
 assert.deepEqual(answerBlocks(''),[{type:'paragraph'}]);
});
