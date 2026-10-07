// Private CDP metadata only. No headers, cookies, bodies, URLs or request IDs
// cross the mirror wire. Only a completed Font response for the current document
// can supply an already-loaded body missing from Page.getResourceContent.
export class LoadedMirrorFonts {
  constructor(){this.sessions=new Map()}
  clear(){this.sessions.clear()}
  removeSession(session){this.sessions.delete(session)}
  observe(message){
    const session=message?.sessionId,p=message?.params;
    if(typeof session!=='string'||!p)return;
    if(message.method==='Network.responseReceived'&&p.type==='Font'&&typeof p.requestId==='string'&&typeof p.loaderId==='string'&&p.loaderId&&typeof p.response?.url==='string'&&p.response.url.length<=8192&&p.response.status>=200&&p.response.status<300){
      let fonts=this.sessions.get(session);if(!fonts){fonts=new Map();this.sessions.set(session,fonts)}
      fonts.delete(p.response.url);fonts.set(p.response.url,{requestId:p.requestId,loaderId:p.loaderId,finished:false});
      // Budget metadata bytes, not page/resource count. An overflow evicts the
      // oldest cached lookup; it never authorizes an origin request.
      let bytes=0;for(const [url,entry]of fonts)bytes+=url.length*2+entry.requestId.length*2+entry.loaderId.length*2+64;
      while(bytes>1024*1024){const [url,entry]=fonts.entries().next().value;bytes-=url.length*2+entry.requestId.length*2+entry.loaderId.length*2+64;fonts.delete(url)}
    }
    if(message.method==='Network.loadingFinished'||message.method==='Network.loadingFailed')for(const [url,entry]of this.sessions.get(session)??[]){if(entry.requestId!==p.requestId)continue;if(message.method==='Network.loadingFailed')this.sessions.get(session).delete(url);else entry.finished=true}
    if(message.method==='Page.frameNavigated'&&!p.frame?.parentId)for(const [url,entry]of this.sessions.get(session)??[])if(entry.loaderId!==p.frame?.loaderId)this.sessions.get(session).delete(url);
    if(message.method==='Target.detachedFromTarget')this.removeSession(p.sessionId??session);
  }
  async read(connection,session,url,document){
    const entry=this.sessions.get(session)?.get(url);if(!entry?.finished||entry.loaderId!==document)return null;
    const result=await connection.send('Network.getResponseBody',{requestId:entry.requestId},session);
    if(this.sessions.get(session)?.get(url)!==entry||!entry.finished||entry.loaderId!==document)throw Error('MP-11: font response changed during read');
    return {base64Encoded:result.base64Encoded,content:result.body};
  }
}
