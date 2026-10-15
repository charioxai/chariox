// MP-08/MP-10/MP-11: event-bound native display documents. CDP capture/input
// retain their query barriers; an admitted native source uses this state only.
import {assertCurrentDocument} from './browser-controller-actions.mjs';
const snapshot=Symbol('native display document');
export class DisplayDocumentPending extends Error {
 constructor(){super('MP-11: display document unavailable while loading');}
}
export class DisplayDocuments {
 constructor(){this.entries=new Map();}
 async target(browser,stored){
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
