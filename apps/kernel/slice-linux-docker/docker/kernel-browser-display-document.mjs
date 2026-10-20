import {captureDocumentOwner} from './browser-controller-frames.mjs';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
// MP-08/MP-10/MP-11: event-bound native display documents. Known page
// captures/state reuse the CDP owner; input admission stays separately guarded.
import {assertCurrentDocument} from './browser-controller-actions.mjs';
const snapshot=Symbol('native display document');
export class DisplayDocumentPending extends Error {
 constructor(){super('MP-11: display document unavailable while loading');}
}
export class DisplayDocuments {
 constructor(){this.entries=new Map();}
 // `wait`: input arriving while a document transition is not yet identified waits for it (bounded) instead of being refused.
 async target(browser,stored,wait=0){
  let entry=this.entries.get(stored.target_id);
  if(entry&&(entry.browser!==browser||entry.connection?.isOpen?.()===false||entry.retired)){this.retire(entry,stored.target_id);entry=null;}
  if(!entry){
   // Install one shared admission before yielding, so parallel viewers cannot
   // attach duplicate observers or overwrite newer navigation events.
   entry={browser,revision:0,document:null,retired:false};this.entries.set(stored.target_id,entry);
   entry.ready=this.admit(entry,stored.target_id).catch(error=>{this.retire(entry,stored.target_id);throw error});
  }
  await entry.ready;
  if(entry.retired||entry.connection.isOpen?.()===false)throw Error('MP-11: display document unavailable');
  for(const until=Date.now()+wait;entry.pending&&!entry.retired&&Date.now()<until;)await new Promise(resolve=>setTimeout(resolve,40));
  if(entry.retired||entry.connection.isOpen?.()===false)throw Error('MP-11: display document unavailable');
  if(entry.pending)throw new DisplayDocumentPending();
  if(!entry.document)throw Error('MP-11: display document unavailable');
  const tab={...stored,document_id:entry.document};
  Object.defineProperty(tab,snapshot,{value:{entry,revision:entry.revision}});
  return tab;
 }
 retire(entry,target){
  entry.retired=true;entry.revision++;entry.off?.();entry.off=null;
  if(this.entries.get(target)===entry)this.entries.delete(target);
 }
 async admit(entry,target){
  const {connection,sessionId}=await entry.browser.resolvePageTarget(target);Object.assign(entry,{connection,sessionId});
  if(entry.retired||this.entries.get(target)!==entry)throw Error('MP-11: display document unavailable');
  entry.off=connection.subscribe(message=>{
   if(message.method==='Chariox.browserDisconnected'||message.method==='Target.targetDestroyed'&&message.params?.targetId===target||message.method==='Target.detachedFromTarget'&&message.params?.sessionId===sessionId){this.retire(entry,target);return;}
   if(message.sessionId!==sessionId)return;
   if(message.method==='Page.frameNavigated'&&!message.params?.frame?.parentId){
    entry.revision++;entry.frame=message.params?.frame?.id;entry.document=message.params?.frame?.loaderId??null;entry.pending=!entry.document;
   }else if(message.method==='Page.frameStartedLoading'&&message.params?.frameId===entry.frame){
    // Refuse captures while a document transition is not yet identified.
    entry.revision++;entry.pending=true;
   }else if(message.method==='Page.loadEventFired'||message.method==='Page.frameStoppedLoading'&&message.params?.frameId===entry.frame){
    // Completion can retain the old loader (cancelled load/download).
    // Resolve it on the event path with the existing revision fence.
    entry.revision++;entry.pending=true;void this.refresh(entry).catch(()=>{this.retire(entry,target);});
   }else if(message.method==='Inspector.detached'){this.retire(entry,target);}
  });
  // A renderer parsing a large document answers Page.getFrameTree only after load. The event-bound frame of this live
  // connection and session is the document that query would return; later navigation events still retire it.
  const known=entry.browser.currentPageFrame?.(target);
  if(known?.connection===connection&&known.sessionId===sessionId&&known.frame?.loaderId){Object.assign(entry,{frame:known.frame.id,document:known.frame.loaderId,pending:false});return;}
  await this.refresh(entry);
 }
 async refresh(entry){
  const revision=entry.revision;
  const {frameTree}=await entry.connection.send('Page.getFrameTree',{},entry.sessionId);
  if(entry.revision!==revision||entry.retired)return;
  const frame=frameTree?.frame;
  entry.frame=frame?.id;entry.document=frame?.loaderId??null;entry.pending=!entry.document;
 }
 current(tab){
  const s=tab[snapshot];return Boolean(s&&!s.entry.retired&&!s.entry.pending&&s.entry.connection.isOpen?.()!==false&&this.entries.get(tab.target_id)===s.entry&&s.entry.revision===s.revision&&s.entry.document===tab.document_id);
 }
 // MP-08/MP-10/MP-11: input pins the requested loader, never the normalized
 // latest loader returned by target(). Mutation of the caller also retires it.
 async guard(browser,stored){
  const target=stored.target_id,document=stored.document_id,tab=await this.target(browser,stored);
  return ()=>stored.target_id===target&&stored.document_id===document&&tab.document_id===document&&this.current(tab);
 }
 connection(tab){const s=tab[snapshot];if(!s)throw Error('MP-11: display document unbound');return s.entry;}
 async validate(tab){
  if(this.current(tab))return true;
  const s=tab[snapshot];
  // A changed binding is always discarded, even if the loader later returns
  // to the old value. A query here preserves the existing stale-reference
  // diagnostic only when the cached loader changed; it cannot re-admit old pixels.
  if(s&&!s.entry.retired&&s.entry.document!==tab.document_id)await assertCurrentDocument(s.entry.connection,s.entry.sessionId,tab.target_id,tab.document_id);
  return false;
 }
 clear(){for(const [target,e] of this.entries)this.retire(e,target);}
}

// MP-08/MP-10/MP-11: native metadata reads use the same document owner as mirrors/captures.
export async function resolveDisplayTarget(host, command, native = false, wait = 0) {
 if(command.generation!==host.generation)throw new UserDomainRefusal('stale_epoch');
 const stored=host.tabs.get(command.tab_id);if(!stored)throw new UserDomainRefusal('stale_reference','display_retired');
 if(native)return host.displayDocuments.target(host.browser,stored,wait);
 const browser=host.browser,target=stored.target_id,generation=host.generation,frame=browser.currentPageFrame?.(target);
 const owner=frame?captureDocumentOwner(browser,{target_id:target,document_id:frame.frame.loaderId}):null;
 const {connection,sessionId}=owner??await browser.resolvePageTarget(target);
 // BROWSER-UX-05: a newly attached page may have a loader event before its
 // full frame owner exists. Reuse that event only in this live session;
 // input admission (fresh) proves the active document from its renderer.
 const cachedDocument=frame||command.fresh?null:browser.documentIdsByTarget?.get(target);
 const document_id=frame?.frame.loaderId??cachedDocument??(await connection.send('Page.getFrameTree',{},sessionId)).frameTree?.frame?.loaderId;
 if(typeof document_id!=='string'||!document_id)throw Error('MD-DISPLAY: document unavailable');
 const current=()=>{
  if(host.browser!==browser||host.generation!==generation||host.tabs.get(command.tab_id)?.target_id!==target)throw new UserDomainRefusal('stale_reference','display_retired');
  if(cachedDocument&&(connection.isOpen?.()===false||'connection' in browser&&browser.connection!==connection||browser.sessionsByTarget&&browser.sessionsByTarget.get(target)!==sessionId||browser.documentIdsByTarget?.get(target)!==document_id))throw new UserDomainRefusal('stale_reference','display_retired');
  owner?.assertCurrent();
 };
 current();
 if(host.tabs.get(command.tab_id).document_id!==document_id){
  const {targetInfo}=await connection.send('Target.getTargetInfo',{targetId:target});current();
  Object.assign(host.tabs.get(command.tab_id),{document_id,url:targetInfo.url,title:targetInfo.title});await host.save();
 }
 current();return {...host.tabs.get(command.tab_id),document_id};
}

export async function nativeStateObservationOptions(host, readOnly) {
 if(!readOnly||!host.chromium?.desktop?.binding?.())return undefined;
 try { const visibleTargetId=await host.tabGroups?.active?.();return typeof visibleTargetId==='string'?{visibleTargetId}:undefined; }
 catch { return undefined; }
}
