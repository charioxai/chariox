// MP-08 / MP-10 / MP-11 H1/H13; synthetic metadata, no benchmark answers.
import assert from 'node:assert/strict'
import test from 'node:test'
import { pathToFileURL } from 'node:url'
const { PassiveHar, observePassiveHar } = await import(process.env.CHARIOX_HAR_MODULE
  ? pathToFileURL(process.env.CHARIOX_HAR_MODULE).href : './passive-har.mjs')
const event = (name, params, sessionId = 's') => ({method: `Network.${name}`, params, sessionId})
const request = (id='r') => event('requestWillBeSent', {requestId:id,wallTime:1,timestamp:1,type:'Document',request:{method:'GET',url:'https://fixture.test/',headers:{}}})
const response = (id='r',extra=true) => event('responseReceived', {requestId:id,hasExtraInfo:extra,response:{status:200,headers:{},mimeType:'text/html'}})
const finish = (id='r') => event('loadingFinished', {requestId:id,timestamp:2,encodedDataLength:20})
const extra = (id='r',accept='text/html') => event('requestWillBeSentExtraInfo',{requestId:id,headers:{Accept:accept,Cookie:'private-fixture',Authorization:'private-fixture'}})

for (const order of [[extra(),request(),response(),finish()],[request(),response(),finish(),extra()]]) {
 test('MP-08 / MP-10 / MP-11 late/early ExtraInfo retires complete request only', () => {
  const har=new PassiveHar(); for (const item of order) har.consume(item)
  assert.equal(har.snapshot().log._capture.activeRequests,0)
  assert.deepEqual(har.snapshot().log.entries[0].request.headers,[{name:'accept',value:'text/html'}])
  assert.equal(JSON.stringify(har.snapshot()).includes('private-fixture'),false)
 })
}
test('MP-08 / MP-10 / MP-11 response without final completion is not flush-admissible', () => {
 const har=new PassiveHar(); har.consume(request()); har.consume(response()); har.consume(extra())
 assert.throws(()=>har.assertComplete(),/active|terminal/)
 har.consume(finish()); har.assertComplete()
 const snapshot=har.snapshot(); snapshot.log.entries.length=0
 assert.equal(har.snapshot().log.entries.length,1)
})
test('MP-08 / MP-10 / MP-11 missing metadata and detached session fail closed', () => {
 const har=new PassiveHar(); har.consume(request()); har.consume(response()); har.consume(finish())
 assert.throws(()=>har.assertComplete(),/ExtraInfo|active/)
 har.retireSession('s'); assert.throws(()=>har.assertComplete(),/session/)
})
test('MP-08 / MP-10 / MP-11 HAR capacity cannot silently drop requests', () => {
 const har=new PassiveHar({maxRequests:1,maxEntries:2}); har.consume(request())
 assert.throws(()=>har.consume(request('r2')),/capacity/)
 assert.throws(()=>har.assertComplete(),/capacity/)
})
test('MP-08 / MP-10 / MP-11 failure remains explicit and requires declared failed-request policy', () => {
 const har=new PassiveHar(); har.consume(request()); har.consume(event('loadingFailed',{requestId:'r',timestamp:2,errorText:'net::ERR_ABORTED'}))
 assert.throws(()=>har.assertComplete(),/failed/)
 har.assertComplete({allowFailedRequests:true})
 assert.equal(har.snapshot().log._capture.failedRequests,1)
})
test('MP-08 / MP-10 / MP-11 redirects and identical IDs in different sessions stay separate', () => {
 const har=new PassiveHar(); har.consume(request()); har.consume(extra('r','text/html;first'))
 har.consume(event('requestWillBeSent',{...request().params,redirectHasExtraInfo:true,redirectResponse:{status:302,headers:{Location:'https://fixture.test/next'}},request:{method:'GET',url:'https://fixture.test/next',headers:{}}}))
 har.consume(response()); har.consume(extra('r','text/html;second')); har.consume(finish())
 for (const e of [request(),response('r',false),finish()]) har.consume({...e,sessionId:'other'})
 har.assertComplete()
 assert.deepEqual(har.snapshot().log.entries.map(e=>e._requestMetadata.redirectIndex),[0,1,0])
 assert.equal(har.snapshot().log._capture.activeRequests,0)
})
test('MP-08 / MP-10 / MP-11 acknowledged observer drain includes late events before detach', async () => {
 let consume,detached=false; const calls=[]
 const observer=await observePassiveHar({sessionIds:['s'],subscribe(fn){consume=fn;return()=>{detached=true}},async send(name){calls.push(name)}})
 consume(request());consume(response());consume(finish())
 const receipt=await observer.close({async drain(){consume(extra());return {settled:true}}})
 assert.equal(detached,true); assert.equal(receipt.log._capture.flushAcknowledged,true)
 assert.deepEqual(calls,['Network.enable','Runtime.getIsolateId'])
 await assert.rejects(observer.close({async drain(){return {settled:true}}}),/closed/)
})
test('MP-08 / MP-10 / MP-11 missing drain acknowledgment rejects final export', async () => {
 let detached=false
 const observer=await observePassiveHar({sessionIds:['s'],subscribe(){return()=>{detached=true}},async send(){}})
 await assert.rejects(observer.close({async drain(){return {settled:false}}}),/drain/)
 assert.equal(detached,true)
})
test('MP-08 / MP-10 / MP-11 close deadline detaches an unresponsive drain',async()=>{
 let detached=false
 const observer=await observePassiveHar({sessionIds:['s'],subscribe(){return()=>{detached=true}},async send(){}})
 await assert.rejects(observer.close({drain:()=>new Promise(()=>{}),timeoutMs:20}),/deadline/)
 assert.equal(detached,true)
})
test('MP-08 / MP-10 / MP-11 total metadata byte budget rejects oversized headers',()=>{
 const har=new PassiveHar({maxBytes:100});assert.throws(()=>har.consume(extra('r','text/html;'+ 'a'.repeat(200))),/byte capacity/)
 assert.throws(()=>har.assertComplete(),/capacity/)
})

const entryBytes = har => Buffer.byteLength(JSON.stringify(har.snapshot().log.entries[0]))
for (const field of ['statusText', 'protocol', 'mimeType']) {
 test(`MP-08 / MP-10 / MP-11 response ${field} cannot exceed metadata budget`, () => {
  const reference=new PassiveHar(); reference.consume(request())
  const har=new PassiveHar({maxBytes:entryBytes(reference)+32}); har.consume(request())
  const before=har.snapshot().log.entries[0]
  const incoming=response('r',false); incoming.params.response[field]='界'.repeat(400)
  assert.throws(()=>har.consume(incoming),/byte capacity/)
  assert.deepEqual(har.snapshot().log.entries[0],before)
  har.consume(finish())
  assert.throws(()=>har.assertComplete(),/byte capacity/)
  assert.throws(()=>har.acknowledgeFlush(),/byte capacity/)
  assert.equal(har.snapshot().log._capture.flushAcknowledged,false)
 })
}

test('MP-08 / MP-10 / MP-11 completed requests accumulate response metadata charges', () => {
 const har=new PassiveHar({maxBytes:2000})
 const first=response('r',false); first.params.response.statusText='a'.repeat(350)
 for (const item of [request(),first,finish()]) har.consume(item)
 har.assertComplete()
 har.consume(request('r2'))
 const second=response('r2',false); second.params.response.statusText='b'.repeat(350)
 const before=har.snapshot().log.entries[1]
 assert.throws(()=>har.consume(second),/byte capacity/)
 assert.deepEqual(har.snapshot().log.entries[1],before)
 assert.throws(()=>har.acknowledgeFlush(),/byte capacity/)
})

for (const method of ['loadingFinished','loadingFailed']) {
 test(`MP-08 / MP-10 / MP-11 ${method} serialized growth is budgeted before mutation`, () => {
  const reference=new PassiveHar(); reference.consume(request()); reference.consume(response('r',false))
  const har=new PassiveHar({maxBytes:entryBytes(reference)})
  har.consume(request()); har.consume(response('r',false))
  const before=har.snapshot().log.entries[0]
  const terminal=method==='loadingFinished' ? finish() : event(method,{requestId:'r',timestamp:2})
  assert.throws(()=>har.consume(terminal),/byte capacity/)
  assert.deepEqual(har.snapshot().log.entries[0],before)
  assert.throws(()=>har.acknowledgeFlush({allowFailedRequests:true}),/byte capacity/)
 })
}

test('MP-08 / MP-10 / MP-11 ExtraInfo merge charges metadata marker growth', () => {
 const reference=new PassiveHar(); reference.consume(request()); reference.consume(response())
 const har=new PassiveHar({maxBytes:entryBytes(reference)+2})
 har.consume(request()); har.consume(response())
 const before=har.snapshot().log.entries[0]
 assert.throws(()=>har.consume(event('requestWillBeSentExtraInfo',{requestId:'r',headers:{}})),/byte capacity/)
 assert.deepEqual(har.snapshot().log.entries[0],before)
 assert.throws(()=>har.acknowledgeFlush(),/byte capacity/)
})

test('MP-08 / MP-10 / MP-11 byte receipt tracks retained entries and queued ExtraInfo exactly', () => {
 const har=new PassiveHar()
 const pending=extra('future')
 har.consume(pending)
 const queued=[{name:'accept',value:'text/html'}]
 for (const item of [request(),response(),extra(),finish(),request('future'),response('future'),finish('future')]) {
  har.consume(item)
  const snapshot=har.snapshot()
  const expected=snapshot.log.entries.reduce((total,entry)=>total+Buffer.byteLength(JSON.stringify(entry)),0)
    +(snapshot.log._capture.unmatchedExtraInfo ? Buffer.byteLength(JSON.stringify(queued)) : 0)
  assert.equal(snapshot.log._capture.retainedMetadataBytes,expected)
 }
 har.acknowledgeFlush()
})
