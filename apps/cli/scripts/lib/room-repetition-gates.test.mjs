// MP-08/MP-10: fail-first acceptance checks for per-cycle leak and identity regressions.
import assert from 'node:assert/strict'
import test from 'node:test'
import { compareCycle, assertSameRoom } from './room-repetition-gates.mjs'
const base = { containers: [], volumes: [], images: [], ownedPids: [], listeners: [], kernel: {rss: 100, fds: 12}, relay: {rss: 50, fds: 8}, diskBytes: 100 }
test('MP-10: leak comparison rejects each resource class and FD/RSS drift', () => {
  assert.deepEqual(compareCycle(base, base), [])
  for (const key of ['containers','volumes','images','ownedPids','listeners']) {
    assert.ok(compareCycle(base, {...base, [key]: ['leak']}).some(x=>x.includes(key)), key)
  }
  assert.ok(compareCycle(base, {...base,kernel:{rss:100,fds:21}}).length)
  assert.ok(compareCycle(base, {...base,relay:{rss:128*1024**2+51,fds:8}}).length)
  assert.ok(compareCycle(base, {...base,diskBytes:64*1024**2+101}).length)
})
test('MP-08: reconnect preserves Room, registry, agents, history and action ledger', () => {
 const a={sessionId:'r',environmentId:'e',generation:1,tabs:[{id:'t'}],agents:[{id:'a'}],history:[],actions:[]}
 assertSameRoom(a,a)
 for(const key of Object.keys(a)) assert.throws(()=>assertSameRoom(a,{...a,[key]:null}),new RegExp(key))
})
test('MP-10: cleanup accepts only the exact already-removed attachment', async () => {
 const {detachOwnedAttachment}=await import('./room-repetition-gates.mjs')
 await detachOwnedAttachment(async()=>{throw Error('attachment `a` was not found')},{},'a')
 for(const message of ['attachment `b` was not found','permission denied','kernel unavailable']) {
  await assert.rejects(detachOwnedAttachment(async()=>{throw Error(message)},{},'a'),new RegExp(message))
 }
})

test('MP-10: reused published ports are foreign; orphan and service listeners remain owned', async () => {
 const {classifyOwnedListeners}=await import('./room-repetition-gates.mjs')
 const reused='LISTEN 0 4096 127.0.0.1:42001 0.0.0.0:* users:(("docker-proxy",pid=21,fd=7))'
 const orphan='LISTEN 0 4096 127.0.0.1:42002 0.0.0.0:* users:(("docker-proxy",pid=22,fd=7))'
 const service='LISTEN 0 128 127.0.0.1:42003 0.0.0.0:* users:(("kernel",pid=23,fd=7))'
 const unrelated='LISTEN 0 128 127.0.0.1:42004 0.0.0.0:* users:(("other",pid=24,fd=7))'
 const rows=['c1|foreign-slice|Up 2 seconds|127.0.0.1:42001->6080/tcp, 127.0.0.1:42003->80/tcp','c2|loops-own-slice|Up 2 seconds|127.0.0.1:42002->6080/tcp']
 const result=classifyOwnedListeners([reused,orphan,service,unrelated],new Set([42001,42002,42003]),rows,'loops-own',[23])
 assert.deepEqual(result.owned,[orphan,service]);assert.deepEqual(result.reused,[{listener:reused,port:42001,containerId:'c1',containerName:'foreign-slice'}])
 assert.deepEqual(classifyOwnedListeners([orphan],new Set([42002]),[],'loops-own',[]).owned,[orphan])
 const ranged=classifyOwnedListeners([reused],new Set([42001]),['c3|foreign-range|Up|127.0.0.1:42000-42009->42000-42009/tcp'],'loops-own',[])
 assert.equal(ranged.reused[0].containerId,'c3')
})

test('MP-08/MP-10: same-slice restart preserves identity; replacement uses its own stable identity', async () => {
 const {assertRestoredMachineIdentity}=await import('./room-repetition-gates.mjs')
 const {createHash}=await import('node:crypto')
 const expected=createHash('sha256').update('slice:slice-2').digest('hex').slice(0,32)
 assertRestoredMachineIdentity('old','old','slice-1','slice-1')
 assert.throws(()=>assertRestoredMachineIdentity('old','changed','slice-1','slice-1'))
 assertRestoredMachineIdentity('old',expected,'slice-1','slice-2')
 assert.throws(()=>assertRestoredMachineIdentity('old','old','slice-1','slice-2'))
})

test('MP-10: foreign host listener reuse is excluded only with process ownership evidence', async () => {
 const {classifyOwnedListeners}=await import('./room-repetition-gates.mjs')
 const host='LISTEN 0 511 127.0.0.1:42001 0.0.0.0:* users:(("node",pid=21,fd=29))'
 const proxy=host.replace('node','docker-proxy')
 assert.deepEqual(classifyOwnedListeners([host],new Set([42001]),[],'own',[],new Set(),new Set([21])).owned,[])
 assert.equal(classifyOwnedListeners([host],new Set([42001]),[],'own',[],new Set([21]),new Set([21])).owned.length,1)
 assert.equal(classifyOwnedListeners([proxy],new Set([42001]),[],'own',[],new Set(),new Set([21])).owned.length,1)
 assert.equal(classifyOwnedListeners([host],new Set([42001]),[],'own',[]).owned.length,1)
})

test('MP-08/MP-10: transient decoded canvas cannot pass stable live-view gate', async () => {
 const {assertStableDecodedViews}=await import('./room-repetition-gates.mjs')
 const frame={connected:true,ready:true,canvasCount:1,streamId:'s',width:1280,height:800,frameHash:1}
 assertStableDecodedViews([[frame],[{...frame,frameHash:2}]],true)
 assert.throws(()=>assertStableDecodedViews([[frame],[{...frame,connected:false,canvasCount:0}]],false))
 assert.throws(()=>assertStableDecodedViews([[frame],[frame]],true))
 assert.throws(()=>assertStableDecodedViews([[frame],[{...frame,streamId:'new'}]],false))
})
