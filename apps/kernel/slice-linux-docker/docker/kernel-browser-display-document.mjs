// MP-08/MP-10/MP-11: event-bound native display documents. CDP capture/input
// retain their query barriers; an admitted native source uses this state only.
import {assertCurrentDocument} from './browser-controller-actions.mjs';
const snapshot=Symbol('native display document');
export class DisplayDocuments {
 constructor(){this.entries=new Map();}
 async target(browser,stored){
  let entry=this.entries.get(stored.target_id);
  if(entry&&(entry.browser!==browser||entry.connection?.isOpen?.()===false||entry.retired)){entry.off?.();this.entries.delete(stored.target_id);entry=null;}
  if(!entry){
   // Install one shared admission before yielding, so parallel viewers cannot
   // attach duplicate observers or overwrite newer navigation events.
   entry={browser,revision:0,document:null,retired:false};this.entries.set(stored.target_id,entry);
   entry.ready=this.admit(entry,stored.target_id).catch(error=>{entry.retired=true;entry.off?.();throw error});
  }
  await entry.ready;
  if(entry.retired||entry.pending||!entry.document||entry.connection.isOpen?.()===false)throw Error('MP-11: display document unavailable');
  const tab={...stored,document_id:entry.document};
  Object.defineProperty(tab,snapshot,{value:{entry,revision:entry.revision}});
  return tab;
 }
 async admit(entry,target){
  const {connection,sessionId}=await entry.browser.resolvePageTarget(target);Object.assign(entry,{connection,sessionId});
  entry.off=connection.subscribe(message=>{
   if(message.method==='Chariox.browserDisconnected'||message.method==='Target.targetDestroyed'&&message.params?.targetId===target||message.method==='Target.detachedFromTarget'&&message.params?.sessionId===sessionId){entry.retired=true;entry.revision++;return;}
   if(message.sessionId!==sessionId)return;
   if(message.method==='Page.frameNavigated'&&!message.params?.frame?.parentId){
    entry.revision++;entry.frame=message.params?.frame?.id;entry.document=message.params?.frame?.loaderId??null;entry.pending=!entry.document;
   }else if(message.method==='Page.frameStartedLoading'&&message.params?.frameId===entry.frame){
    // Refuse captures while a document transition is not yet identified.
    entry.revision++;entry.pending=true;
   }else if(message.method==='Page.loadEventFired'||message.method==='Page.frameStoppedLoading'&&message.params?.frameId===entry.frame){
    // Completion can retain the old loader (cancelled load/download).
    // Resolve it on the event path with the existing revision fence.
    entry.revision++;entry.pending=true;void this.refresh(entry).catch(()=>{entry.retired=true;});
   }else if(message.method==='Inspector.detached'){entry.retired=true;entry.revision++;}
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
 clear(){for(const e of this.entries.values()){e.retired=true;e.off?.();}this.entries.clear();}
}
