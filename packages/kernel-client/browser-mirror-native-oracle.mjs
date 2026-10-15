// MP-08/MP-10/MP-11 M0: drill-only, read-only attachment to the owned public browser.
// The kernel still launches/owns Chromium. Never use this seam for user input.
import assert from 'node:assert/strict'
import {captureOracleSnapshot} from './browser-mirror-oracle.mjs'
export async function connectNativeOracle(endpoint){
 const url=new URL(endpoint);assert(url.protocol==='ws:'&&['127.0.0.1','localhost','[::1]'].includes(url.hostname)&&!url.username&&!url.password,'MP-11: oracle CDP must be owned loopback')
 const socket=new WebSocket(endpoint),pending=new Map();let next=0
 socket.addEventListener('message',event=>{const message=JSON.parse(event.data);const work=pending.get(message.id);if(!work)return;pending.delete(message.id);clearTimeout(work.timer);message.error?work.reject(Error('MP-10: read-only CDP refused')):work.resolve(message.result)})
 await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',()=>reject(Error('MP-10: owned CDP unavailable')),{once:true})})
 const send=(method,params={},sessionId)=>new Promise((resolve,reject)=>{const id=++next,timer=setTimeout(()=>{pending.delete(id);reject(Error('MP-10: read-only CDP timeout'))},30000);pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})}))})
 return {async capture(tab){
  const {targetInfos}=await send('Target.getTargets'),targets=targetInfos.filter(t=>t.type==='page'&&t.url===tab.url)
  let sessionId
  for(const target of targets){const candidate=(await send('Target.attachToTarget',{targetId:target.targetId,flatten:true})).sessionId
   const frame=(await send('Page.getFrameTree',{},candidate)).frameTree.frame
   if(frame.loaderId===tab.document_id){assert(!sessionId,'MP-10: ambiguous native document');sessionId=candidate}else await send('Target.detachFromTarget',{sessionId:candidate})
  }
  assert(sessionId,'MP-10: bound native document required')
  const attached=[sessionId]
  try{
   const frame=async()=>(await send('Page.getFrameTree',{},sessionId)).frameTree.frame
   assert.equal((await frame()).loaderId,tab.document_id,'MP-10: native document binding before snapshot')
   const snapshot=await captureOracleSnapshot((m,p)=>send(m,p,sessionId))
   const frames=new Map(),parents=new Map(),queue=[],bindings=[]
   const index=(snap,sid)=>{for(let i=0;i<snap.documents.length;i++)frames.set(snap.strings[snap.documents[i].frameId],{snap,sid,index:i})}
   const tree=(node,parent=null)=>{if(parent)parents.set(node.frame.id,parent);if(!frames.has(node.frame.id)&&!queue.includes(node.frame.id))queue.push(node.frame.id);for(const child of node.childFrames??[])tree(child,node.frame.id)}
   index(snapshot,sessionId);tree((await send('Page.getFrameTree',{},sessionId)).frameTree)
   for(let at=0;at<queue.length;at++){
    assert(at<128,'MP-10: native frame evidence limit exceeded')
    const id=queue[at];if(frames.has(id))continue
    const parent=frames.get(parents.get(id));assert(parent,'MP-10: parent frame evidence missing')
    const {backendNodeId}=await send('DOM.getFrameOwner',{frameId:id},parent.sid)
    const doc=parent.snap.documents[parent.index],owner=doc.nodes.backendNodeId.indexOf(backendNodeId)
    assert(owner>=0,'MP-10: native frame owner evidence missing')
    // A frame with no layout cannot paint. Visible orphan frames still fail in oracleRecords.
    if(!doc.layout.nodeIndex.includes(owner))continue
    const current=(await send('Target.getTargets')).targetInfos
    assert(current.some(t=>t.type==='iframe'&&t.targetId===id),'MP-10: out-of-process frame evidence unavailable')
    const sid=(await send('Target.attachToTarget',{targetId:id,flatten:true})).sessionId;attached.push(sid)
    const before=(await send('Page.getFrameTree',{},sid)).frameTree
    assert.equal(before.frame.id,id,'MP-10: native child frame binding')
    const child=await captureOracleSnapshot((m,p)=>send(m,p,sid))
    const after=(await send('Page.getFrameTree',{},sid)).frameTree
    assert.equal(after.frame.loaderId,before.frame.loaderId,'MP-10: native child document changed during snapshot')
    bindings.push({sid,id,loader:after.frame.loaderId})
    ;(parent.snap.externalFrames??={})[parents.get(id)+':'+owner]=child
    index(child,sid);tree(after,parents.get(id))
   }
   for(const b of bindings){const now=(await send('Page.getFrameTree',{},b.sid)).frameTree.frame;assert(now.id===b.id&&now.loaderId===b.loader,'MP-10: native child evidence changed during capture')}
   assert.equal((await frame()).loaderId,tab.document_id,'MP-10: native document binding after snapshot')
   return snapshot
  }finally{for(const sid of attached.reverse())await send('Target.detachFromTarget',{sessionId:sid}).catch(()=>{})}
 },close(){socket.close();for(const work of pending.values()){clearTimeout(work.timer);work.reject(Error('MP-10: oracle closed'))}pending.clear()}}
}
