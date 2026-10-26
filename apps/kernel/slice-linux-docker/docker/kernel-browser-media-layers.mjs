// MP-08/MP-10/MP-11: private compositor evidence for media exclusions.
// A DOM tag is a candidate, never proof. Each node must occur in its own
// document's current compositor tree; an unattached OOPIF supplies no evidence.
import {slotOf,localId} from './kernel-browser-mirror2-frames.mjs';
const mediaTags=new Set(['VIDEO','CANVAS','OBJECT','EMBED']);
export async function mediaLayerEvidence(mirror,stream,boxes){
 const groups=[];
 for(const box of boxes){
  // A composited (rotated, translucent, filtered) element is excluded by its on-screen box like any other media.
  if(box.media!==true)continue;
  const slot=slotOf(box.id),entry=slot?[...stream.frames.values()].find(e=>e.slot===slot):null;
  const world=slot?entry?.child:stream.world,loader=slot?entry?.loaderId:stream.document_id;
  if(!world||!loader||await mirror.frames.loaderId(world)!==loader)continue;
  let group=groups.find(g=>g.connection===world.connection&&g.sessionId===world.sessionId);
  if(!group){group={connection:world.connection,sessionId:world.sessionId,nodes:[]};groups.push(group)}
  group.nodes.push({box,world,loader,key:slot?localId(box.id):box.id});
 }
 const evidence=[];
 for(const group of groups){
  const {connection,sessionId,nodes}=group;let layers=null,resolve,timer;
  const ready=new Promise(r=>resolve=r);
  const off=connection.subscribe(m=>{
   if(m.sessionId===sessionId&&m.method==='LayerTree.layerTreeDidChange'&&Array.isArray(m.params?.layers)){
    layers=m.params.layers.length<=4096?m.params.layers.filter(l=>l.drawsContent===true&&Number.isSafeInteger(l.backendNodeId)&&l.backendNodeId>0&&Number.isFinite(l.width)&&l.width>0&&Number.isFinite(l.height)&&l.height>0):[];
    if(layers.length)resolve();
   }
  });
  try{
   await connection.send('LayerTree.enable',{},sessionId);
   // Enabling LayerTree alone may not commit a static visible page. Await
   // its own frame ticks without a screenshot's temporary surface resize.
   const tick=await connection.send('Runtime.evaluate',{contextId:nodes[0].world.contextId,awaitPromise:true,returnByValue:true,
    expression:'new Promise(resolve=>{const timer=setTimeout(()=>resolve(false),500);requestAnimationFrame(()=>requestAnimationFrame(()=>{clearTimeout(timer);resolve(true)}))})'},sessionId);
   if(tick.exceptionDetails||tick.result?.value!==true)continue;
   if(!layers?.length)await Promise.race([ready,new Promise(r=>timer=setTimeout(r,500))]);
   if(!layers?.length)continue;
   for(const {box,world,loader,key} of nodes){
    if(await mirror.frames.loaderId(world)!==loader)continue;
    let objectId;
    try{
     const reply=await connection.send('Runtime.evaluate',{expression:`${world.ref}.mediaNode(${JSON.stringify(key)})`,contextId:world.contextId,returnByValue:false},sessionId);
     objectId=reply.result?.objectId;if(!objectId||reply.exceptionDetails)continue;
     const {node}=await connection.send('DOM.describeNode',{objectId,depth:0},sessionId);
     const layer=layers.find(l=>l.backendNodeId===node?.backendNodeId);
     if(!layer||node.nodeType!==1||!mediaTags.has(node.nodeName)||await mirror.frames.loaderId(world)!==loader)continue;
     evidence.push({id:box.id,box:[...(box.clip??box.box)],backend_node_id:node.backendNodeId,loader_id:loader,layer_width:layer.width,layer_height:layer.height});
    }finally{if(objectId)await connection.send('Runtime.releaseObject',{objectId},sessionId).catch(()=>{})}
   }
  }catch{ /* No trusted layer evidence: its pixels remain in the exact compare. */ }
  finally{clearTimeout(timer);off();await connection.send('LayerTree.disable',{},sessionId).catch(()=>{})}
 }
 return evidence;
}
