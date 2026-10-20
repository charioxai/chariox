// MP-08/MP-10/MP-11: public 519 adapter over the owners' private region leases.
// No client geometry, native handle, permission path, or separate frame pump.
import {reserveDisplay} from './kernel-browser-display-admission.mjs';
import {randomUUID} from 'node:crypto';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
import {NativeWindowRegionCapture} from './kernel-browser-window-region.mjs';
import {createRegionDisplay} from './kernel-browser-region-display.mjs';
import {inputHostTab} from './kernel-browser-input.mjs';
import {lateInputAck,markDispatched} from './kernel-browser-input-queue.mjs';
import {popupActionId} from './kernel-browser-popup-evidence.mjs';
import {slowMark} from './kernel-browser-timing.mjs';

const refuse=()=>{throw new UserDomainRefusal('stale_reference','region_retired');};
// The kernel Chrome's own UI (band, popups): window leases, not page regions.
const chromeUi=target=>target?.kind==='browser_chrome'||target?.kind==='browser_auxiliary';
function checkedTarget(target) {
 const keys=target?.kind==='region'?['kind','mirror_subscription_id','document_id','sequence','region_id']:
  target?.kind==='browser_chrome'||target?.kind==='browser_window'?['kind','window_generation']:
  target?.kind==='browser_auxiliary'?['kind','window_generation','auxiliary_id']:[];
 if(!keys.length||Object.keys(target).some(key=>!keys.includes(key))||keys.some(key=>!(key in target)))refuse();
 return target;
}
export function subscribeRegion(host,command,scope,signal,options={}) {
 return reserveDisplay(host,()=>subscribeReservedRegion(host,command,scope,signal,options));
}
async function subscribeReservedRegion(host,command,scope,signal,{Capture=NativeWindowRegionCapture}={}) {
 assertNotCancelled(signal);
 const target=checkedTarget(command.target);
 // State advertisement is not admission: a terminal can guess a window epoch.
 // Only managed lockdown proves Chrome cannot replace this unmasked band
 // with same-size fullscreen page pixels.
 if(chromeUi(target)&&host.chromium?.policy!=='managed')refuse();
 if(command._agent_input||command.focused_agent||command.generation!==host.generation)refuse();
 if(!Array.isArray(command.codecs)||command.codecs.length>8||!command.codecs.includes('png')||!command.codecs.includes('avc1.420033')||
  !Number.isInteger(command.bitrate)||command.bitrate<500_000||command.bitrate>64_000_000||![1,2].includes(command.device_scale_factor)||host.displays.size>=8)refuse();
 // Protocol 560: the owner's window display asks no Tab or renderer anything.
 if(target.kind==='browser_window')return (await import('./kernel-browser-window-display.mjs')).subscribeBrowserWindow(host,command,scope,signal,Capture===NativeWindowRegionCapture?{}:{Capture});
 const tab=await host.displayTarget(command),policy=host.protection,generation=host.generation;
 if(tab.tab_id!==command.tab_id)refuse();
 let lease,args,placement,offset={x:0,y:0},overlayTab=null;
 const windows=host.chromeWindows?.current();
 // A band or popup stream is bound to the window's identity only: a page-only geometry change (r54) leaves it current.
 const tabbed=windows?.tabbed,window=tabbed&&target.kind!=='region'&&tabbed.isBandCurrent?{...tabbed,isCurrent:tabbed.isBandCurrent}:tabbed;
 if(!window||window.role!=='tabbed'||window.browser_generation!==generation||!window.isCurrent())refuse();
 if(target.kind==='region') {
  if(!host.mirror.v2?.regions)refuse();
  const {kind,...reference}=target;
  lease=await host.mirror.v2.regions.resolve({...reference,generation},scope,{signal});
  try {
   if(lease.tab_id!==tab.tab_id||lease.document_id!==tab.document_id||!await lease.isCurrent())refuse();
   const scale=lease.device_scale_factor,r=lease.rect;
   if(scale!==window.scale)refuse();
   const page=window.page_rect;
   if(!page||typeof window.isPageCurrent!=='function'||!await window.isPageCurrent())refuse();
   // A hidden mirror can retain its document/geometry while another Tab paints
   // in this SAME X window. Only the browser owner's native active-tab observer
   // can admit that readback; focus-emulated visibility is not authority.
   if(typeof host.arbiter?.isActiveTab!=='function'||!await host.arbiter.isActiveTab(tab,window))refuse();
   // Stay INSIDE the canonical clip; the codec's even dimensions never expose
   // adjacent pixels. Keep the offset for the existing mirror input resolver.
   const left=Math.ceil(r.x*scale),top=Math.ceil(r.y*scale),width=2*Math.floor((Math.min(page.width,Math.floor((r.x+r.width)*scale))-left)/2),height=2*Math.floor((Math.min(page.height,Math.floor((r.y+r.height)*scale))-top)/2);
   if(width<2||height<2)refuse();
   offset={x:left/scale-r.x,y:top/scale-r.y};
   const element=lease.element_offset;
   if(!element||![element.x,element.y].every(v=>Number.isFinite(v)&&v>=0))refuse();
   // The mirror clip origin is relative to the opaque element. Capture rounds
   // inward again; paint uses both offsets, input uses only this last rounding.
   placement=Object.freeze({x:element.x+offset.x,y:element.y+offset.y});
   if(![placement.x,placement.y].every(v=>Number.isFinite(v)&&v>=0))refuse();
   args={surface:'page',window:{id:window.xid,width:window.bounds.width,height:window.bounds.height},
    rect:{left:page.left+left,top:page.top+top,width,height},contentHeight:page.height,contentRect:page,
    attestationState:lease.attestationState,
    // The owner's fresh viewport query proves geometry and the active target
    // together; a second active() query repeats the same native admission.
    cachedCurrent:typeof window.isPageCurrentCached==='function'?()=>lease.isAlive()&&window.isPageCurrentCached(tab.target_id):undefined,
    isCurrent:()=>lease.isAlive()&&window.isCurrent(),validateCurrent:async()=>window.isCurrent()&&await window.isPageCurrent(tab.target_id)&&await lease.isCurrent()};
   args.occluded=async()=>await host.chromeWindows?.inWindowOverNow?.(args.rect)===true;
  }catch(error){await lease.close();throw error;}
 } else {
  if(!Number.isSafeInteger(target.window_generation)||target.window_generation!==window.window_generation)refuse();
  // Protocol 530: one of the band window's own popups (menu, dropdown, extension popup, bubble).
  const popup=target.kind==='browser_auxiliary'?host.chromeWindows.auxiliary(target.auxiliary_id):null;
  const current=()=>host.chromium.policy==='managed'&&window.isCurrent()&&(!popup||popup.isCurrent());
  args={...(popup?host.chromeWindows.auxiliaryRegion(popup):host.chromeWindows.band(window)),isCurrent:current};
  // Chrome UI drawn in the window over the page (a dialog's page overlay, the omnibox dropdown) shows
  // Chrome's ACTIVE Tab around or after it: that Tab's masks and fence, on every readback.
  if(args.inWindow){
   const active=await host.arbiter?.activeTab?.();
   if(!active||typeof host.arbiter?.isActiveTab!=='function'||!await host.arbiter.isActiveTab(active,window))refuse();
   overlayTab=active;args.validateCurrent=async()=>current()&&await host.arbiter.isActiveTab(active,window);
   if(host.tabGroups?.nativeStateSupported?.()===true)args.cachedCurrent=()=>current()&&host.tabGroups.cachedActive()===active.target_id;
  }
  const binding_id=`host-chrome-region-${randomUUID()}`;
  lease={binding_id,generation,document_id:`chrome-window-${generation}-${window.window_generation}${popup?`-${popup.auxiliary_id}`:''}`,sequence:window.window_generation,
   isAlive:current,isCurrent:async()=>current(),close:async()=>{}};
 }
 return createRegionDisplay({host,command,scope,signal,target,lease,args,window,tab,policy,generation,placement,offset,overlayTab,Capture});
}
// A visible page is admitted from the cached native state (a parsing page answers renderer reads late and an unparkable
// wait here would outlast the viewer's request); every fence before the dispatch still reads live through the input guard.
const streamAdmitted=stream=>stream.regionTarget.kind==='visible_page'&&stream.regionFrameCurrent?stream.regionFrameCurrent():stream.regionCurrent();
function inputStream(host,command,scope) {
 if(command._agent_input||command.focused_agent||command.generation!==host.generation)refuse();
 const stream=[...host.displays.values()].find(s=>s.binding_id===command.binding_id);
 if(!stream||stream.observed_by!==scope||
  (stream.tab_id!==command.tab_id&&!(chromeUi(stream.regionTarget)&&command.region_document_id&&host.tabs.has(command.tab_id)))||stream.expires<=Date.now()||
  stream.regionLease.document_id!==(command.region_document_id??command.document_id)||stream.regionLease.sequence!==command.sequence||
  !stream.regionLease.isAlive()||!stream.regionWindow.isCurrent())refuse();
 return stream;
}
// Read-only pre-ledger adaptation: the owner arbiter observes the ACTIVE page.
// The ordinary Rust actor ledger still admits every resulting input action.
export async function regionInputTarget(host,command,scope,signal) {
 const stream=inputStream(host,command,scope);assertNotCancelled(signal);
 if(stream.regionTarget.kind==='browser_window')refuse();
 // MP-11: this only names the stream's own tab/document. A visible page needs no renderer read for that (a parsing
 // page answers late); the input that follows runs the fresh regionCurrent, input guard and final fences.
 if(!await streamAdmitted(stream))refuse();
 if(['region','visible_page'].includes(stream.regionTarget.kind))return {tab_id:stream.tab_id,document_id:stream.regionLease.document_id};
 if(typeof host.arbiter?.resolveDisplayInput!=='function')refuse();
 // This window epoch is kernel-issued private adapter data, not an extra
 // public DisplayInput field or a client window claim.
 const target=await host.arbiter.resolveDisplayInput({...command,window_generation:stream.regionWindow.window_generation},scope,{signal,stream});
 if(!target||!host.tabs.has(target.tab_id)||!target.document_id)refuse();
 return {tab_id:target.tab_id,document_id:target.document_id,region_document_id:stream.regionLease.document_id};
}
export async function regionInput(host,command,scope,signal) {
 const browserAtStart=host.browser;
 const stream=inputStream(host,command,scope);assertNotCancelled(signal);
 // Protocol 560: the window display is view-only; the mirror and the band own input.
 if(stream.regionTarget.kind==='browser_window')refuse();
 if(!await slowMark(host.timing,'input_wait region_admit',streamAdmitted(stream)))refuse();
 if(stream.regionTarget.kind==='visible_page')return (await import('./kernel-browser-page-display.mjs')).inputVisiblePage(host,stream,command,scope,signal);
 if(chromeUi(stream.regionTarget)) {
  if(typeof host.arbiter?.input!=='function')refuse();
  // vidlat C10: like page input, a band input marks its stream's input epoch/time and a numeric
  // input_dispatch at the native action boundary (band frames are then graded as input-triggered).
  const tab=stream.regionSource.tab?.tab_id??stream.tab_id;
  const onDispatch=()=>{
   host.inputChangedAt?.set(tab,performance.now());
   host.timing.event?.('input_dispatch',{at:performance.timeOrigin+performance.now(),input:host.inputCount=(host.inputCount??0)+1,kind:{click:1,text:2,key:3,scroll:4}[command.input?.kind]??0});
   host.inputEpochs?.set(tab,(host.inputEpochs.get(tab)??0)+1);
  };
  return host.arbiter.input({stream,command,scope,signal,onDispatch});
 }
 const input=command.input;
 if(!['click','scroll'].includes(input?.kind)||input.button!==undefined||!Number.isInteger(input.x)||!Number.isInteger(input.y)||input.x<0||input.y<0||input.x>=stream.css_width||input.y>=stream.css_height)refuse();
 const resolved=await stream.regionLease.resolveInput({...input,sequence:command.sequence,x:input.x+stream.regionOffset.x,y:input.y+stream.regionOffset.y},scope,{signal});
 const guard=resolved.guard;
 resolved.guard=async()=>{if(!await stream.regionCurrent())refuse();await guard?.();};
 if(host.dispatchRegionInput)return host.dispatchRegionInput(stream,resolved,command,signal);
 const tab=await host.displayTarget(command);
 if(tab.document_id!==stream.regionLease.document_id)refuse();
 let dispatched=false;
 try { await host.sampleLane(tab).run('input',()=>host.popupEvidence.capture(host.browser,tab,popupActionId(command),beginDispatch=>
  inputHostTab(host.browser,tab,{kind:'mirror'},{signal,resolveMirror:async()=>resolved,onDispatch:()=>{
   dispatched=true;
   host.inputEpochs.set(tab.tab_id,(host.inputEpochs.get(tab.tab_id)??0)+1);host.inputChangedAt.set(tab.tab_id,performance.now());stream.regionSource.wake();return beginDispatch();
  }}))); }
 catch(error){if(dispatched)markDispatched(error);if((dispatched&&!lateInputAck(error)||['browser_action_cancelled','stale_document_reference'].includes(error?.code))&&(host.isBrowserLive?.(browserAtStart)??host.browser===browserAtStart))await host.stop();throw error;}
 return host.observe(await host.reconcile().catch(error=>{if(error?.code==='browser_cdp_timeout')return host.tabState();throw markDispatched(error);}),null,scope);
}
