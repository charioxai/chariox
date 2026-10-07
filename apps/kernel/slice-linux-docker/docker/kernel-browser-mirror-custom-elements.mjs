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
  const describe=async object=>{const {node}=await connection.send('DOM.describeNode',{objectId:object.value.objectId,depth:0,pierce:true},sessionId);if(!node||node.nodeType!==1||!Number.isSafeInteger(node.backendNodeId))throw Error('MP-11: custom host metadata unavailable');return {backend:node.backendNodeId,closed:(node.shadowRoots??[]).some(root=>root.shadowRootType!=='open'&&root.shadowRootType!=='user-agent')}};
  const metadata=await Promise.all(objects.map(describe));
  await Promise.all(objects.map((object,i)=>connection.send('Runtime.callFunctionOn',{objectId:object.value.objectId,functionDeclaration:'function(closed){globalThis.__charioxMirror.admitCustomHost(this,closed);return true}',arguments:[{value:metadata[i].closed}],returnByValue:true},sessionId).then(r=>{if(r.exceptionDetails||r.result?.value!==true)throw Error('MP-11: custom host admission unavailable')})));
  return {fingerprint:JSON.stringify(metadata),release,verify:async()=>{const current=await Promise.all(objects.map(describe));if(JSON.stringify(current)!==JSON.stringify(metadata))throw Error('MP-11: custom host changed during mirror frame')}};
 }catch(error){await release();throw error}
}
