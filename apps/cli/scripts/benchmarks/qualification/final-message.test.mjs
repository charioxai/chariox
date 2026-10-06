// MP-08 / MP-10 / MP-11 H2; use the shared helper selected by coordinator.
import assert from 'node:assert/strict'
import test from 'node:test'
import { pathToFileURL } from 'node:url'
import { readFile } from 'node:fs/promises'
import { sha256 } from './admission.mjs'
const modulePath=process.env.CHARIOX_FRAGMENT_HELPERS_MODULE
if (!modulePath) throw new Error('MP-08 / MP-10 / MP-11 requires built shared fragment helper (r2next dependency)')
const {assembleSessionHistoryFinalMessage: assemble}=await import(pathToFileURL(modulePath).href)
const entry=(index,text,start,total,metadata={})=>({entry_index:index,fragment_start:start,fragment_end:start+Array.from(text).length,total_chars:total,
 entry:{kind:'provider_output',text,provider_run_id:'run-1',merge_key:'final',timestamp_ms:index,...metadata}})
test('MP-08 / MP-10 / MP-11 H2 split emoji/combining/non-Latin text stays byte-exact once',()=>{
 const text='Résumé 👩‍💻\né 日本語.\n',chars=Array.from(text),n=chars.length
 const summary=entry(9,chars.slice(0,3).join(''),0,n)
 const chunks=[entry(9,chars.slice(0,7).join(''),0,n),entry(9,chars.slice(7,10).join(''),7,n),entry(9,chars.slice(10).join(''),10,n)]
 const actual=assemble({lifecycle:'completed',summary},[chunks[2],chunks[1],chunks[0],chunks[1],entry(8,'commentary',0,10,{merge_key:'commentary'})])
 assert.deepEqual(Buffer.from(actual),Buffer.from(text)); assert.notEqual(actual,'.')
})
test('MP-08 / MP-10 / MP-11 H2 combined preview is not appended as original output',()=>{
 const summary=entry(9,'Éva ',0,11)
 assert.equal(assemble({lifecycle:'completed',summary},[entry(10,'👋 日本語.\n',0,7),entry(9,'Éva ',0,4)]),'Éva 👋 日本語.\n')
})
for (const lifecycle of ['failed','cancelled','open']) test(`MP-08 / MP-10 / MP-11 H2 rejects ${lifecycle}`,()=>{
 assert.throws(()=>assemble({lifecycle,summary:entry(1,'fixture',0,7)},[]),/settlement/)
})
test('MP-08 / MP-10 / MP-11 H2 missing final identity, gaps and foreign run cannot reconstruct',()=>{
 const summary=entry(9,'first',0,11)
 assert.throws(()=>assemble({lifecycle:'completed'},[entry(9,'.',0,1)]),/summary/)
 assert.throws(()=>assemble({lifecycle:'completed',summary},[entry(9,'first',0,11),entry(9,'last',7,11)]),/incomplete/)
 assert.throws(()=>assemble({lifecycle:'completed',summary},[entry(9,'first tail!',0,11,{provider_run_id:'foreign'})]),/incomplete/)
})
test('MP-08 / MP-10 / MP-11 H2 records external helper byte identity',async()=>{assert.match(sha256(await readFile(modulePath)),/^[a-f0-9]{64}$/)})
