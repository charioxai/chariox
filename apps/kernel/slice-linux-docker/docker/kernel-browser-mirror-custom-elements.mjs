// Custom tag names do not imply a closed shadow tree. Native CDP metadata
// admits ordinary light DOM; closed/unknown roots remain opaque. Recheck before
// committing a frame and on every chunk credit, never trusting page claims.
import {randomUUID} from 'node:crypto';
export async function inspectMirrorCustomElements(world){
 const {connection,sessionId,contextId}=world,objectGroup='mirror-custom-'+randomUUID();
 const release=()=>connection.send('Runtime.releaseObjectGroup',{objectGroup},sessionId).catch(()=>{});
 try{
  const result=await connection.send('Runtime.evaluate',{expression:'globalThis.__charioxMirror.customHosts()',contextId,objectGroup,returnByValue:false},sessionId);
  if(result.exceptionDetails)throw Error('MP-11: custom host observation unavailable');
  if(result.result?.value===null){await release();return {fingerprint:'[]',verify:async()=>{},release:async()=>{}}}
  if(!result.result?.objectId)throw Error('MP-11: native custom host references unavailable');
  const properties=await connection.send('Runtime.getProperties',{objectId:result.result.objectId,ownProperties:true},sessionId);
  const count=properties.result.find(p=>p.name==='length')?.value?.value;
  if(!Number.isSafeInteger(count)||count<0||count>100000)throw Error('MP-11: custom host working set bounds');
  const objects=properties.result.filter(p=>/^(0|[1-9][0-9]*)$/.test(p.name)).sort((a,b)=>Number(a.name)-Number(b.name));
  if(objects.length!==count||objects.some((p,i)=>Number(p.name)!==i||p.value?.subtype!=='node'||!p.value.objectId))throw Error('MP-11: invalid native custom host');
  const describe=async object=>{
   const {node}=await connection.send('DOM.describeNode',{objectId:object.value.objectId,depth:0,pierce:true},sessionId);
   if(!node||node.nodeType!==1||!Number.isSafeInteger(node.backendNodeId))throw Error('MP-11: custom host metadata unavailable');
   const root=(node.shadowRoots??[]).find(root=>root.shadowRootType!=='open'&&root.shadowRootType!=='user-agent');
   let flow=null;
   if(root?.backendNodeId){
    // Read geometry only through the native closed-root reference. Never read
    // text, attributes, URLs or serialize the closed tree. An inline host with
    // one block child has block-in-inline flow which an atomic IMG cannot keep.
    const resolved=await connection.send('DOM.resolveNode',{backendNodeId:root.backendNodeId,executionContextId:contextId,objectGroup},sessionId);
    if(!resolved.object?.objectId)throw Error('MP-11: opaque flow reference unavailable');
    const measured=await connection.send('Runtime.callFunctionOn',{objectId:resolved.object.objectId,functionDeclaration:`function(){
     if(this.childNodes.length!==1||this.firstChild.nodeType!==1)return null;
     const child=this.firstChild,host=this.host,h=getComputedStyle(host),s=getComputedStyle(child),a=host.getBoundingClientRect(),b=child.getBoundingClientRect();
     if(h.display!=='inline'||s.display!=='block'||s.position!=='static'||s.float!=='none'||s.transform!=='none'||s.writingMode!=='horizontal-tb'||Math.max(...['x','y','width','height'].map(k=>Math.abs(a[k]-b[k])))>.01)return null;
     const top=parseFloat(s.marginTop),bottom=parseFloat(s.marginBottom);if(!Number.isFinite(top)||!Number.isFinite(bottom))return null;
     return {display:'block',width:a.width+'px',height:a.height+'px','margin-top':top+'px','margin-bottom':bottom+'px'};
    }`,returnByValue:true},sessionId);
    if(measured.exceptionDetails||!Object.hasOwn(measured.result??{},'value'))throw Error('MP-11: opaque flow unavailable');
    flow=measured.result.value;
   }
   return {backend:node.backendNodeId,closed:Boolean(root),flow};
  };
  const metadata=await Promise.all(objects.map(describe));
  await Promise.all(objects.map((object,i)=>connection.send('Runtime.callFunctionOn',{objectId:object.value.objectId,functionDeclaration:'function(closed,flow){globalThis.__charioxMirror.admitCustomHost(this,closed,flow);return true}',arguments:[{value:metadata[i].closed},{value:metadata[i].flow}],returnByValue:true},sessionId).then(r=>{if(r.exceptionDetails||r.result?.value!==true)throw Error('MP-11: custom host admission unavailable')})));
  return {fingerprint:JSON.stringify(metadata),release,verify:async()=>{const current=await Promise.all(objects.map(describe));if(JSON.stringify(current)!==JSON.stringify(metadata))throw Error('MP-11: custom host changed during mirror frame')}};
 }catch(error){await release();throw error}
}
