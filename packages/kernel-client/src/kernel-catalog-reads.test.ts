// MP-08/MP-10/MP-11: supplementary transport coalescing regression.
import test from 'node:test'
import assert from 'node:assert/strict'
import {KernelCatalogReads} from './kernel-catalog-reads.js'
function deferred<T>(){let resolve!:(value:T)=>void;const promise=new Promise<T>(r=>{resolve=r});return{promise,resolve}}
test('MP-08/MP-10/MP-11 identical pending catalogs share one transport read; scopes and mutations do not',async()=>{
 const reads=new KernelCatalogReads(),work=deferred<number>();let sent=0
 const send=()=>{sent++;return work.promise},request={GetProviderCatalog:{provider:'codex',account_profiles:{codex:'a'},execution_location:{kind:'local'}}}
 const a=reads.run(request,'owner-A/kernel-A',send),b=reads.run(request,'owner-A/kernel-A',send)
 assert.equal(sent,1)
 reads.run(request,'owner-B/kernel-A',send);assert.equal(sent,2)
 reads.run({KernelBrowser:{command:{op:'navigate'}}},'owner-A/kernel-A',send);reads.run({KernelBrowser:{command:{op:'navigate'}}},'owner-A/kernel-A',send);assert.equal(sent,4)
 work.resolve(7);assert.deepEqual(await Promise.all([a,b]),[7,7]);await reads.run(request,'owner-A/kernel-A',async()=>++sent);assert.equal(sent,5)
})
test('MP-11 failure and clear never leave a stale catalog promise reusable',async()=>{
 const reads=new KernelCatalogReads(),request={GetProviderCatalog:{}};let n=0
 await assert.rejects(reads.run(request,'scope',async()=>{throw Error('refused')}))
 assert.equal(await reads.run(request,'scope',async()=>++n),1)
 const pending=deferred<number>();const old=reads.run(request,'scope',()=>pending.promise);reads.clear();const fresh=reads.run(request,'scope',async()=>++n);assert.equal(await fresh,2);pending.resolve(0);await old
 assert.equal(await reads.run(request,'scope',async()=>++n),3)
})


test('MP-08/MP-10 catalogs wait for initial mirror paint while writes proceed and duplicate reads coalesce',async()=>{
 const reads=new KernelCatalogReads(),release=reads.hold();let sent=0
 const request={GetProviderCatalog:{}}
 const a=reads.run(request,'scope',async()=>++sent),b=reads.run(request,'scope',async()=>++sent)
 assert.equal(sent,0)
 assert.equal(await reads.run({SubmitPrompt:{}},'scope',async()=>++sent),1)
 release();assert.deepEqual(await Promise.all([a,b]),[2,2]);assert.equal(sent,2)
})
test('MP-11 clearing a display hold cancels deferred reads before any changed authority can write',async()=>{
 const reads=new KernelCatalogReads(),release=reads.hold();let sent=0
 const a=reads.run({GetProviderCatalog:{}},'old-scope',async()=>++sent)
 reads.clear();release();await assert.rejects(a,/catalog read cancelled/);assert.equal(sent,0)
 assert.equal(await reads.run({GetProviderCatalog:{}},'new-scope',async()=>++sent),1)
})
