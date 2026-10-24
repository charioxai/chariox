// MP-08/MP-11: execute the live drill's fixture setup without starting its stack.
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
const source=readFileSync(new URL('./live-owner-context-transfer-drill.mjs',import.meta.url),'utf8');
const setup=source.slice(source.indexOf('  const utf16='),source.indexOf('  const metadata='));
assert(setup.startsWith('  const utf16='),'MP-08/MP-11 fixture setup must exist');
const {safeScript,canary}=vm.runInNewContext(setup+';({safeScript,canary})',{Buffer});
test('MP-08/MP-11 ordinary live fixtures are fully classified UTF-8',()=>{
 const text=new TextDecoder('utf-8',{fatal:true}).decode(safeScript);
 assert.equal(text,"Write-Output 'MP-08 ordinary 🦀'\n");
 assert(!text.includes('\0'));
});
test('MP-08/MP-11 the explicit wide-text fixture remains Vault-routed',()=>{
 assert.deepEqual([...canary.subarray(0,2)],[255,254]);
 assert(canary.subarray(2).toString('utf16le').includes('API_KEY'));
});
