// MP-08 / MP-09 / MP-10 / MP-11: retain only delivery and verification classification.
// Request bodies, response fields other than the boolean predicate, headers and tokens never leave CDP.
export function validateCardVerification(value, origin) {
  if (!value || typeof value !== 'object' || Object.keys(value).sort().join(',') !== 'accepted_json_pointer,challenge_id,url'
    || !/^[a-zA-Z0-9._:-]{1,128}$/.test(value.challenge_id)
    || typeof value.accepted_json_pointer !== 'string' || !/^\/(?:[^~]|~[01]){1,127}$/.test(value.accepted_json_pointer)) throw new Error('Invalid browser verification condition');
  let url;try {url=new URL(value.url)}catch {throw new Error('Invalid browser verification URL')}
  if (url.origin !== origin || url.username || url.password || url.search || url.hash) throw new Error('Browser verification requires an exact same-origin URL without credentials or query');
  return {challenge_id:value.challenge_id,url:url.href,accepted_json_pointer:value.accepted_json_pointer};
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
    this.unsubscribe=connection.subscribe(message=>this.observe(message));
  }
  close() {this.closed=true;this.unsubscribe();this.request=null}
  observe(message) {
    if(this.closed || this.receipt?.result==='accepted' || message.sessionId!==this.sessionId)return;
    const p=message.params ?? {};
    if(message.method==='Network.requestWillBeSent') {
      if(p.redirectResponse && this.request?.id===p.requestId) {this.finish('failed');return}
      if(p.frameId!==this.frame.id || p.loaderId!==this.frame.loaderId || p.request?.method!=='POST' || p.request.url!==this.config.url)return;
      this.request={id:p.requestId,status:null};
      this.receipt={challenge_id:this.config.challenge_id,request_id:p.requestId,input_delivered:true,http_status:null,result:'delivered'};
    } else if(this.request?.id===p.requestId) {
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
