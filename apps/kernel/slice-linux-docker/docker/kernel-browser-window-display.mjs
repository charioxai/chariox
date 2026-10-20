// MP-08/MP-10/MP-11 (protocol 560): the owner's view of their own Chrome window's page area. The display is bound
// to the window, not to a Tab or document: Tab create, switch and navigation never retire it, and admission asks
// the page renderer nothing (the window generation and page rect come from the owner's native window state).
// Masks stay per document and are measured only while a Vault fill target exists.
import {randomUUID} from 'node:crypto';
import {NativeWindowRegionCapture} from './kernel-browser-window-region.mjs';
import {startNativeReadback,nativeReadbackFailure} from './kernel-browser-native-readback.mjs';
import {regionProtection} from './kernel-browser-region-attestation.mjs';
import {createRegionDisplay} from './kernel-browser-region-display.mjs';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
import {NavigationHold} from './kernel-browser-navigation-hold.mjs';

const retry=()=>Object.assign(Error('MP-11: fill-target capture unavailable; retry'),{code:'fill_capture_retry'});
const filled=host=>Boolean(host.protection?.targets?.length||host.browser?.fillTargets?.size);

export function windowCaptureArgs(window){
 const p=window.page_rect;
 return {window:{id:window.xid,width:window.bounds.width,height:window.bounds.height},rect:{left:p.left,top:p.top,width:p.width-p.width%2,height:p.height-p.height%2},
  contentRect:{left:p.left,top:p.top,width:p.width,height:p.height},contentHeight:p.height};
}
// No page session is attached: the browser-level connection carries only the lifetime events the base class needs.
export class WindowPageCapture extends NativeWindowRegionCapture {
 constructor({host,...options}){
  super({...options,surface:'page'});
  this.host=host;
  // Frames of a Tab's new, still unpainted document are held back (see NavigationHold); a popup over the page still holds too.
  const popup=this.occluded;
  this.navigation=new NavigationHold({active:()=>host.tabGroups?.active?.().catch(()=>null),release:()=>this.wake(true)});
  this.occluded=async()=>this.attested&&await this.navigation.holding()||popup?.();
 }
 onCdp(message){
  if(message.method==='Chariox.browserDisconnected')this.fence();
  this.navigation.event(message);
 }
 async close(){this.navigation.close();return super.close();}
 protection(){
  const host=this.host,scale=this.scale,{pageLeft,pageTop}=this.region;let inner=null,innerTarget=null,revision=0;
  const front=async()=>{const target=await host.tabGroups?.active?.().catch(()=>null);return {target,tab:[...host.tabs.values()].find(t=>t.target_id===target)};};
  // With a fill target the front Tab's page session measures it (the same fence as a page region). That fence
  // is taken here, before the readback it will judge: a fence made after the readback would refuse it forever.
  const prepare=async()=>{
   if(!filled(host)){inner=innerTarget=null;return;}
   const {target,tab}=await front();
   if(!tab)return;
   if(innerTarget!==target){
    const {connection,sessionId}=await host.browser.resolvePageTarget(target);
    inner=regionProtection({connection,sessionId,policy:host.protection,browser:host.browser,tab,scale},pageLeft,pageTop);innerTarget=target;
   }
   if(!inner.guard)await inner.refresh();
  };
  return {guard:null,beforeAt:-Infinity,
   async refresh(){
    const seen=revision;await prepare();
    if(seen!==revision)throw Error('MP-11: native window fence retired');
    this.guard={};this.beforeAt=performance.timeOrigin+performance.now();
   },
   retire(){revision++;this.guard=null;inner?.retire();inner=innerTarget=null;},
   tracker:()=>({targets:filled(host)}),
   // Without a fill target nothing is measured; a readback older than a fence is retried.
   regions:async function(raw){
    if(!this.guard||raw.captured_ms<this.beforeAt)throw retry();
    if(!filled(host)){inner=innerTarget=null;return [];}
    const {target}=await front();
    if(!inner||innerTarget!==target)throw retry();
    return inner.regions(raw);
   }};
 }
 async start(){
  if(!this.valid())throw Error('MP-11: native window lease retired');
  this.off=this.connection.subscribe(m=>this.onCdp(m));
  try{
   await this.connection.presentationFence?.recoverFailed?.();
   const region=this.region;
   this.regions=this.protection();
   await this.regions.refresh();
   this.poolRoot=await this.createRasterPool();
   startNativeReadback(this,{width:region.width,height:region.height,window_region:region.window_region});
   if(!await this.waitForCurrentSample())throw nativeReadbackFailure(this,'MP-08: native window readback unavailable');
   if(!this.valid())throw Error('MP-11: native window lease retired');
   this.attested=true;return this;
  }catch(error){await this.close();throw error;}
 }
}

export async function subscribeBrowserWindow(host,command,scope,signal,{Capture=WindowPageCapture}={}){
 assertNotCancelled(signal);
 const target=command.target,window=host.chromeWindows?.current()?.tabbed,generation=host.generation;
 if(command._agent_input||command.focused_agent||command.generation!==generation)throw new UserDomainRefusal('stale_reference');
 if(!window||window.role!=='tabbed'||window.browser_generation!==generation||!window.isCurrent()||target.window_generation!==window.window_generation)throw new UserDomainRefusal('stale_reference');
 const p=window.page_rect;
 if(!p||p.top<=0||!Number.isSafeInteger(window.xid)||window.xid<=1)throw new UserDomainRefusal('stale_reference');
 const {rect,...geometry}=windowCaptureArgs(window);
 const connection=await host.browser.ensureConnection();
 // One lease per window epoch: a new Tab or a navigation is not an epoch change.
 const document_id=`chrome-window-${generation}-${window.window_generation}-page`;
 const tab={tab_id:command.tab_id,target_id:null,document_id};
 const lease={binding_id:`host-window-page-${randomUUID()}`,generation,document_id,sequence:window.window_generation,
  isAlive:()=>window.isCurrent(),isCurrent:async()=>window.isCurrent(),close:async()=>{}};
 const occluded=async()=>host.chromeWindows.inWindowOverNow({left:rect.left,top:rect.top,width:rect.width,height:rect.height});
 const args={...geometry,rect,host,isCurrent:()=>window.isCurrent(),occluded,
  validateCurrent:async()=>window.isCurrent(),publishCurrent:async()=>window.isCurrent()};
 return createRegionDisplay({host,command,scope,signal,target,lease,args,window,tab,policy:host.protection,generation,connection,Capture});
}
