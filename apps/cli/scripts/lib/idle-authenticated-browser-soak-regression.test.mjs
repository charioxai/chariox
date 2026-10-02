// MP-07 / MP-08 / MP-10: execute the failing frozen-D harness seams without a browser.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'
const source = await readFile(new URL('./idle-authenticated-browser-soak-runtime.mjs', import.meta.url), 'utf8')
const checkpointBody = source.split('  async function checkpoint(label) {')[1].split('    if (fixture.authenticatedRequests <= beforeRequests)')[0]
async function checkpoint(label) {
  const calls = [], fixture = {baseUrl:'http://fixture', authenticatedRequests:7}
  let generation = 1
  const context = {
    label, fixture, calls, counters:{authenticatedSessionChecks:3}, options:{healthIntervalSeconds:30},
    controllerIdentity:{}, markerDigest:'digest',
    controller:{child:{pid:1}, async request(method, params) {
      calls.push({method, params})
      if (method === 'health') return {}
      if (method === 'browser.reconcile') return {tabs:[{url:'http://fixture', target_id:'tab', document_id:generation}]}
      if (method === 'browser.snapshot') {assert.equal(params.document_id, generation, 'snapshot must target post-wait document'); return ['Authenticated synthetic session', 'digest']}
    }},
    assertControllerReady:value=>value, assertOwnedAlive:async()=>{},
    waitForFreshAuthenticatedRequest:async(_fixture, baseline, timeout)=>{
      calls.push({baseline, timeout});fixture.authenticatedRequests++;generation++
    }, Math, JSON,
  }
  await vm.runInNewContext(`(async()=>{${checkpointBody}})()`, context)
  return calls
}
test('MP-08 / MP-10 final/restart request must start after the checkpoint begins', async()=>{
  for (const label of ['initial','restart','final']) {
    const calls=await checkpoint(label)
    assert.equal(calls.find(x=>x.baseline!=null).baseline,7)
  }
})
test('MP-08 / MP-10 reconcile carries desktop dimensions and refreshes the document after waiting', async()=>{
  const calls=await checkpoint('periodic')
  const reconciles=calls.filter(x=>x.method==='browser.reconcile')
  assert.equal(reconciles.length,2)
  for(const entry of reconciles) assert.equal(entry.params.viewport.desktop_pixel_width,800)
  assert.equal(calls.find(x=>x.baseline!=null).timeout,32000)
})
test('MP-08 / MP-10 restart publishes fresh grace checkpoint then flushes Chromium before termination',async()=>{
  const prefix=source.split('      if (!controlledRestartCompleted && elapsed >= restartAt) {')[1].split('        const stopped =')[0]
  const events=[]
  class Socket {
    constructor(){queueMicrotask(()=>this.onopen())}
    send(text){assert.equal(JSON.parse(text).method,'Browser.close');events.push('flush');queueMicrotask(()=>this.onmessage({data:'{"id":1}'}))}
    close(){}
  }
  const context={events,chromium:{},allocation:{debugPort:1},
    raceActive:value=>value,checkpoint:async label=>{events.push(label)},allowOwnedTermination:()=>events.push('allow'),
    fetch:async()=>({json:async()=>({webSocketDebuggerUrl:'ws://fixture'})}),WebSocket:Socket,
    sleep:async()=>{},setTimeout,clearTimeout,JSON,Promise}
  await vm.runInNewContext(`(async()=>{${prefix}})()`,context)
  assert.deepEqual(events,['before_restart','allow','flush'])
})
test('MP-08 / MP-10 direct Chromium launch preserves the recorded process identity',()=>{
  const body=source.split('function spawnSoakChromium(')[1].split('\n}\n')[0]
  const context={spawnLogged:(name,binary,args)=>({name,binary,args})}
  const launch=vm.runInNewContext(`(function spawnSoakChromium(${body}\n})`,context)
  assert.equal(launch({profileRoot:'/tmp/profile',debugPort:1,url:'http://fixture',cwd:'/source',logsRoot:'/tmp/logs'}).binary,'/usr/lib/chromium/chromium')
})
