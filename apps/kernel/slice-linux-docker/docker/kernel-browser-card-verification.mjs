// MP-08 / MP-09 / MP-10 / MP-11: retain only delivery and verification classification.
// Request bodies, response fields other than the boolean predicate, headers and tokens never leave CDP.
import {randomUUID} from 'node:crypto';

function bindVerificationInput(name, resolveAnchor) {
  const original=this;
  let sequence=0, field=null;
  const delivered=new Map();
  const acknowledge=event=>{
    if (!event.isTrusted || event.target?.ownerDocument!==document || !event.target.isConnected) return;
    const anchor=resolveAnchor ? resolveAnchor() : original;
    if (!anchor?.isConnected || !event.composedPath().includes(anchor)) return;
    const target=event.target;
    if (event.type==='input' && ['INPUT','TEXTAREA'].includes(target.nodeName) && !target.readOnly && !['checkbox','radio','hidden','button','submit'].includes(target.type)) {
      field=target;
      if(target.value.length<=512)delivered.set(target,target.value);else delivered.delete(target);
    }
    for(const [input,value] of delivered)if(!input.isConnected || !anchor.contains(input) || input.value!==value)delivered.delete(input);
    if(delivered.size>64) {delivered.clear();field=null;return}
    if (!field?.isConnected || !delivered.has(field) || field.ownerDocument!==document || !field.value || field.value.length>512) return;
    // This private CDP binding is never an observation/event-journal result.
    // Keep the delivered value inside the helper, solely to correlate the POST.
    // Segmented codes submit one aggregate in document order, even when the
    // person edits the boxes out of order. Only include trusted delivered input.
    const segments=[...delivered.keys()].filter(input=>input.form===field.form && input.value.length===1)
      .sort((a,b)=>a.compareDocumentPosition(b)&Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1);
    const aggregate=segments.length>1 ? segments.map(input=>delivered.get(input)).join('') : '';
    globalThis[name](JSON.stringify({sequence:++sequence,value:field.value.length===1 && aggregate ? aggregate : field.value}));
  };
  const kinds=['input','click','keydown','submit'];
  for(const kind of kinds)document.addEventListener(kind,acknowledge,true);
  return ()=>{for(const kind of kinds)document.removeEventListener(kind,acknowledge,true)};
}

export function validateCardVerification(value, origin) {
  if (!value || typeof value !== 'object' || Object.keys(value).sort().join(',') !== 'accepted_json_pointer,challenge_id,url'
    || !/^[a-zA-Z0-9._:-]{1,128}$/.test(value.challenge_id)
    || typeof value.accepted_json_pointer !== 'string' || !/^\/(?:[^~]|~[01]){1,127}$/.test(value.accepted_json_pointer)) throw new Error('Invalid browser verification condition');
  let url;try {url=new URL(value.url)}catch {throw new Error('Invalid browser verification URL')}
  if (url.origin !== origin || url.username || url.password || url.search || url.hash) throw new Error('Browser verification requires an exact same-origin URL without credentials or query');
  return {challenge_id:value.challenge_id,url:url.href,accepted_json_pointer:value.accepted_json_pointer};
}

function hasSubmittedValue(body, input, headers) {
  if (typeof body!=='string' || body.length>65536 || typeof input!=='string' || !input)return false;
  const contentType=Object.entries(headers ?? {}).find(([key])=>key.toLowerCase()==='content-type')?.[1] ?? '';
  if (typeof contentType!=='string')return false;
  const mediaType=contentType.split(';',1)[0].trim().toLowerCase();
  if(mediaType==='application/x-www-form-urlencoded')return [...new URLSearchParams(body).values()].some(value=>value===input);
  if(mediaType==='multipart/form-data') {
    const boundary=/(?:^|;)\s*boundary=(?:"([^"]+)"|([^;\s]+))/i.exec(contentType);
    const delimiter=boundary?.[1] ?? boundary?.[2];
    if(!delimiter || delimiter.length>70 || /[\r\n]/.test(delimiter))return false;
    const parts=body.split('--'+delimiter);
    if(parts.length>258 || !parts.at(-1).startsWith('--'))return false;
    return parts.slice(1,-1).some(part=>{
      const split=part.indexOf('\r\n\r\n');
      if(!part.startsWith('\r\n') || split<0 || !part.endsWith('\r\n'))return false;
      const disposition=part.slice(2,split).split('\r\n').find(line=>/^content-disposition:/i.test(line));
      return disposition && /^content-disposition:\s*form-data\s*;/i.test(disposition)
        && /;\s*name=/i.test(disposition) && !/;\s*filename\*?=/i.test(disposition) && part.slice(split+4,-2)===input;
    });
  }
  let parsed;try {parsed=JSON.parse(body)}catch{return false}
  const pending=[[parsed,0]];let visited=0;
  while(pending.length && ++visited<=256) {
    const [value,depth]=pending.pop();
    if ((typeof value==='string' || typeof value==='number') && String(value)===input)return true;
    if (value && typeof value==='object' && depth<8)for(const child of Object.values(value))pending.push([child,depth+1]);
  }
  return false;
}

function booleanAt(value, pointer) {
  for (const key of pointer.slice(1).split('/').map(key=>key.replaceAll('~1','/').replaceAll('~0','~'))) {
    if (!value || typeof value !== 'object' || !Object.hasOwn(value,key)) return false;
    value=value[key];
  }
  return value === true;
}

export class BrowserCardVerification {
  static async create(connection, sessionId, frame, config) {
    const watch=new BrowserCardVerification(connection,sessionId,frame,config);
    try {await connection.send('Network.enable',{},sessionId);return watch}
    catch {watch.close();throw new Error('Browser verification observation could not start')}
  }
  constructor(connection,sessionId,frame,config) {
    this.connection=connection;this.sessionId=sessionId;this.frame=frame;this.config=config;
    this.receipt=null;this.request=null;this.closed=false;
    this.bindingName='charioxVerification_'+randomUUID().replaceAll('-','_');
    this.contextId=null;this.inputSequence=0;this.consumedSequence=0;this.inputAt=0;this.cleanupId=null;this.inputValue=null;
    this.unsubscribe=connection.subscribe(message=>this.observe(message));
  }
  async bindInput(objectId,contextId,resolverId) {
    if(this.contextId!==null)return;
    await this.connection.send('Runtime.addBinding',{name:this.bindingName,executionContextId:contextId},this.sessionId);
    const result=await this.connection.send('Runtime.callFunctionOn',{objectId,functionDeclaration:String(bindVerificationInput),arguments:[{value:this.bindingName},...(resolverId ? [{objectId:resolverId}] : [])],returnByValue:false},this.sessionId);
    if(result.exceptionDetails || !result.result?.objectId)throw new Error('Browser verification input acknowledgement could not start');
    this.cleanupId=result.result.objectId;this.contextId=contextId;
  }
  close() {
    if(this.closed)return;
    this.closed=true;this.unsubscribe();this.request=null;this.inputValue=null;
    const cleanup=this.cleanupId ? this.connection.send('Runtime.callFunctionOn',{objectId:this.cleanupId,functionDeclaration:'function(){this()}',returnByValue:true},this.sessionId).catch(()=>{}).finally(()=>this.connection.send('Runtime.releaseObject',{objectId:this.cleanupId},this.sessionId).catch(()=>{})) : Promise.resolve();
    void cleanup.finally(()=>this.connection.send('Runtime.removeBinding',{name:this.bindingName},this.sessionId).catch(()=>{}));
  }
  observe(message) {
    if(this.closed || this.receipt?.result==='accepted' || message.sessionId!==this.sessionId)return;
    const p=message.params ?? {};
    if(message.method==='Runtime.bindingCalled') {
      if(p.name!==this.bindingName || p.executionContextId!==this.contextId || typeof p.payload!=='string' || p.payload.length>2048)return;
      let input;try {input=JSON.parse(p.payload)}catch{return}
      if(Number.isSafeInteger(input.sequence) && input.sequence>this.inputSequence && typeof input.value==='string' && input.value.length>0 && input.value.length<=512) {
        this.inputSequence=input.sequence;this.inputAt=Date.now();this.inputValue=input.value;
      }
      return;
    }
    if(message.method==='Network.requestWillBeSent') {
      if(p.redirectResponse && this.request?.id===p.requestId) {this.finish('failed');return}
      if(p.frameId!==this.frame.id || p.loaderId!==this.frame.loaderId || p.request?.method!=='POST' || p.request.url!==this.config.url
        || p.hasUserGesture!==true || this.inputSequence<=this.consumedSequence || Date.now()-this.inputAt>5000
        || !hasSubmittedValue(p.request.postData,this.inputValue,p.request.headers))return;
      this.consumedSequence=this.inputSequence;this.inputValue=null;
      this.request={id:p.requestId,status:null};
      this.receipt={challenge_id:this.config.challenge_id,request_id:p.requestId,input_delivered:true,http_status:null,result:'delivered'};
    } else if(this.request && this.request.id===p.requestId) {
      if(message.method==='Network.responseReceived') {
        if(p.response?.url!==this.config.url || !Number.isInteger(p.response.status) || p.response.status<100 || p.response.status>599) {this.finish('failed');return}
        this.request.status=p.response.status;
        if(p.response.status<200 || p.response.status>=300)this.finish('rejected');
      } else if(message.method==='Network.loadingFailed')this.finish('failed');
      else if(message.method==='Network.loadingFinished')void this.completed(p).catch(()=>this.finish('failed',p.requestId));
    }
  }
  finish(result,id=this.request?.id) {
    if(this.closed || this.request?.id!==id || !id)return;
    this.receipt={challenge_id:this.config.challenge_id,request_id:id,input_delivered:true,http_status:this.request.status,result};
    if(result!=='delivered')this.request=null;
  }
  async completed(event) {
    const request=this.request;
    if(!request || request.status===null || event.encodedDataLength>65536) {this.finish('failed',event.requestId);return}
    const response=await this.connection.send('Network.getResponseBody',{requestId:request.id},this.sessionId);
    if(typeof response.body!=='string' || response.body.length>131072) {this.finish('failed',request.id);return}
    const body=response.base64Encoded?Buffer.from(response.body,'base64').toString('utf8'):response.body;
    let accepted=false;try {accepted=booleanAt(JSON.parse(body),this.config.accepted_json_pointer)}catch {}
    // An old response cannot accept a newer attempt or a replacement document.
    const current=(await this.connection.send('Page.getFrameTree',{},this.sessionId)).frameTree?.frame;
    if(current?.id!==this.frame.id || current.loaderId!==this.frame.loaderId) {this.finish('failed',request.id);return}
    this.finish(accepted?'accepted':'rejected',request.id);
  }
}
