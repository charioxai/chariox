// MP-08/MP-11: transient field identities for MODEL captures, never owner video.
// No typed text, hashes or registrations survive this measurement.
export function typedSensitiveField(type='',autocomplete='') {
 return type.toLowerCase()==='password'||autocomplete.toLowerCase().split(/\s+/).some(token=>token.startsWith('cc-')||['one-time-code','current-password','new-password'].includes(token));
}
export function typedBrowserTargets(snapshot,entry,targetId,documentId) {
 const strings=snapshot.strings,targets=[];
 for(const document of snapshot.documents??[]) {
  const nodes=document.nodes;
  for(let index=0;index<nodes.nodeName.length;index++) {
   if(!['input','textarea'].includes(strings[nodes.nodeName[index]].toLowerCase()))continue;
   const attributes=nodes.attributes[index]??[],attrs=new Map();
   for(let i=0;i<attributes.length;i+=2)attrs.set(strings[attributes[i]].toLowerCase(),strings[attributes[i+1]].toLowerCase());
   if(!typedSensitiveField(attrs.get('type'),attrs.get('autocomplete')))continue;
   targets.push({target_id:targetId,document_id:documentId,node_ref:entry.prefix+'backend:'+nodes.backendNodeId[index],frame_id:strings[document.frameId],model_typed:true});
  }
 }
 return targets;
}
