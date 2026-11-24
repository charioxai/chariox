// MP-08/MP-10/MP-11: one region stream lifetime for page crops and Chrome UI.
import {randomUUID} from 'node:crypto';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
import {DisplayStream,PortableEncoder} from './kernel-browser-display.mjs';
const refuse=()=>{throw new UserDomainRefusal('stale_reference');};
const chromeUi=t=>['browser_chrome','browser_auxiliary'].includes(t?.kind);
// Row 341 (A), allocated 1015: the whole-page video (the visible page, not a navigation's Tab display preview) of a viewer
// that accepts stills at its own density comes to rest at that density; element crops and previews keep the page density.
export const regionViewerScale=(target,command)=>target?.kind==='visible_page'&&command.tab_display!==true&&command.viewer_density_stills===true?{viewer_scale:command.device_scale_factor}:{};
// Row 487: a region's capture source, its render hold and its lease checks, opened for one window binding. A display opens
// one at subscribe; a visible page that follows a resize opens another for the new window and swaps it in (rebindRegion).
export async function openRegionSource({host,target,lease,args,window,tab,policy,generation,overlayTab=null,connection:windowConnection=null,Capture}){
 let source,releaseRender;
 try{
  const pageTab=overlayTab??tab;
  // A window-scoped display has no page: the browser-level connection carries it and no page session is resolved.
  const cached=windowConnection?null:host.browser.currentPageFrame?.(pageTab.target_id);
  const {connection,sessionId}=windowConnection?{connection:windowConnection}:cached?.frame.loaderId===pageTab.document_id?cached:await host.browser.resolvePageTarget(pageTab.target_id);
  const scale=window.scale,sourceKind={browser_chrome:1,region:2,browser_auxiliary:3,visible_page:4,browser_window:5}[target.kind]??0;
  const timedLease=check=>check&&(async(lease_phase=1)=>{
   const at=performance.timeOrigin+performance.now();
   try{return await check();}finally{host.timing.event?.('native_live_lease',{at,source_kind:sourceKind,lease_phase});}
  });
  // Browser UI survives page navigation; the frame document is its window epoch.
  // A dialog overlay's frames belong to the window epoch too; its page document fences it separately.
  const sourceTab=chromeUi(target)?{...pageTab,document_id:lease.document_id}:pageTab;
  const {cachedCurrent,validateCurrent:validateArg,publishCurrent:publishArg,...captureArgs}=args;
  const validateCurrent=timedLease(validateArg),publishCurrent=timedLease(publishArg);
  // Native pixels do not keep a hidden RenderWidget producing animation.
  // Share the target-local wake with mirrors, without selecting a physical tab.
  if(target.kind==='visible_page'||target.kind==='region')releaseRender=await host.browser.inputCapture.hold(connection,sessionId);
  source=new Capture({...captureArgs,validateCurrent,publishCurrent,...(overlayTab?{pageDocument:overlayTab.document_id}:{}),display:host.chromium.display,pid:window.pid??host.chromium.browserPid??host.chromium.child?.pid,connection,sessionId,tab:sourceTab,scale,
   hostScale:host.chromium.scale??1,policy,browser:host.browser,frames:id=>host.browser.frameSession?.(id,connection),
   screenshot:()=>host.displayScreenshot(pageTab,null,false),allowed:p=>host.protection===p&&!p.unknown&&host.generation===generation,timing:host.timing});
  source.captureKind=sourceKind;
  // No scrim behind the "dialog" (a darker theme dimmed the toolbar): the kernel learns the toolbar again.
  try{await source.start();}catch(error){if(overlayTab&&error?.scrim!==undefined)host.chromeWindows.relearnToolbar?.();throw error;}
  return {source,releaseRender,scale,sourceKind,validateCurrent,publishCurrent,cachedCurrent};
 }catch(error){
  // The lease stays with the caller: it reads it (retired or not) before closing it.
  try{await source?.close();}finally{await releaseRender?.();}
  throw error;
 }
}
// The checks a stream reads its source's binding through (input, frame delivery); a rebind installs the new source's.
function regionChecks(host,stream,lease,window,{sourceKind,validateCurrent,publishCurrent,cachedCurrent}){
 return {regionCurrent:validateCurrent?()=>validateCurrent(2):lease.isCurrent,
  // A key or wheel notch is admitted from the same event-bound snapshot as frame publication (fresh read on a miss);
  // clicks and drags keep the fresh read.
  regionInputCurrent:publishCurrent?(input=>['key','scroll'].includes(input?.kind)?publishCurrent(2):stream.regionCurrent()):undefined,
  // Input keeps the fresh regionCurrent path. Only frame delivery reuses the
  // source-publication snapshot, bounded by native owner and document events.
  regionFrameCurrent:cachedCurrent?(()=>{host.timing.event?.('native_cached_lease',{source_kind:sourceKind,lease_phase:2});return lease.isAlive()&&window.isCurrent()&&cachedCurrent();}):undefined};
}
export async function createRegionDisplay({host,command,scope,signal,target,lease,args,window,tab,policy,generation,placement,offset={x:0,y:0},overlayTab=null,connection:windowConnection=null,Capture}){
 let opened,stream;
 try {
  opened=await openRegionSource({host,target,lease,args,window,tab,policy,generation,overlayTab,connection:windowConnection,Capture});
  const {source,scale}=opened;
  assertNotCancelled(signal);
  if(host.protection!==policy||!await lease.isCurrent()||!window.isCurrent()||args.validateCurrent&&!await args.validateCurrent())refuse();
  const result={source_kind:'region',binding_id:lease.binding_id,document_id:lease.document_id,sequence:lease.sequence,
   generation,subscription_id:`host-display-${randomUUID()}`,codec:'avc1.420033',bitrate:command.bitrate,
   device_scale_factor:scale,css_width:source.region.css_width,css_height:source.region.css_height,
   ...(placement?{region_offset:placement}:{})};
  // Row 332: the visible page's whole-page video sizes its rate by its picture (areaBitrate); element crops keep theirs.
  stream=new DisplayStream({...result,...(target.kind==='visible_page'?{whole_page:true}:{}),...regionViewerScale(target,command),tab_id:tab.tab_id,observed_by:scope,relay_binary:command.codecs.includes('chariox-relay-binary-v96'),
   // Crops use whole-video dependencies; an eight-row encoder cannot divide
   // every valid even crop (including a two-device-pixel opaque tile).
   dependencies:command.codecs.includes('chariox-video-dependencies-v1'),stripes:false},
   {timing:host.timing,encoder:new PortableEncoder()});
  Object.assign(stream,{regionLease:lease,regionSource:source,regionTarget:target,regionWindow:window,regionOffset:offset,regionPlacement:placement,regionRelease:opened.releaseRender});
  const checks=regionChecks(host,stream,lease,window,opened);
  stream.regionCurrent=checks.regionCurrent;if(checks.regionInputCurrent)stream.regionInputCurrent=checks.regionInputCurrent;if(checks.regionFrameCurrent)stream.regionFrameCurrent=checks.regionFrameCurrent;
  const close=stream.close.bind(stream);let closing,offRetire;
  // Teardown closes the source and lease the stream holds now (a rebind replaced the first ones).
  stream.close=()=>closing??=(async()=>{
   if(typeof offRetire==='function')offRetire();host.displays.delete(stream.subscription_id);stream.rebinding?.settle(false);
   try{
    const results=await Promise.allSettled([close(),stream.regionSource.close()]);
    const errors=results.filter(result=>result.status==='rejected').map(result=>result.reason);
    if(errors.length)throw new AggregateError(errors,'MD-DISPLAY: region teardown failed');
   }finally{try{await stream.regionRelease?.();}finally{await stream.regionLease.close();}}
  })();
  if(target.kind==='region')lease.attachDisplay(stream);else host.displays.set(stream.subscription_id,stream);
  const unwatch=target.kind==='browser_chrome'&&typeof source.windows==='function'?host.chromeWindows.watch?.(()=>source.windows(),undefined,fn=>source.onWindowsChanged?.(fn)):null;
  // The band's own frames tell when a browser-modal dialog dims the window (no other signal exists).
  const offBand=target.kind==='browser_chrome'&&typeof source.subscribe==='function'?source.subscribe(frame=>host.chromeWindows.observeBand?.(frame?.raw)):null;
  // Row 487: a visible page that asked for resize_rebind outlives the resize's own window retire: its delivery waits while
  // the kernel admits the same Tab on the new window (rebindRegion). Any other retire removes it, as before.
  const offWindow=host.chromeWindows.onRetire?.(({reason}={})=>{
   if(stream.regionWindow.isCurrent())return;
   if(reason==='viewport_resize'&&stream.resizeRebind===true&&target.kind==='visible_page'&&typeof stream.rebindSource==='function'&&!stream.rebinding){stream.rebinding=rebindHold(host,stream);return;}
   void host.removeDisplay(stream.subscription_id).catch(()=>{});
  });
  offRetire=()=>{unwatch?.();offBand?.();offWindow?.();};
  host.armDisplayExpiry(stream);return result;
 }catch(error){
  // Startup may fail after its admitted display retires; check before cleanup closes the lease.
  let current;
  try{current=window.isCurrent()&&await lease.isCurrent();}
  finally{if(stream)await stream.close();else if(opened){try{await opened.source.close();}finally{try{await opened.releaseRender?.();}finally{await lease.close();}}}else await lease.close();}
  if(!current)refuse();
  throw error;
 }
}
// Row 487: delivery waits for the rebind (bounded); a rebind that does not land in REBIND_MS removes the display as today.
const REBIND_MS=3000;
function rebindHold(host,stream){
 let settle;const done=new Promise(resolve=>{settle=resolve;});
 const timer=setTimeout(()=>{hold.settle(false);void host.removeDisplay(stream.subscription_id).catch(()=>{});},REBIND_MS);timer.unref?.();
 const hold={done,settled:false,settle:ok=>{if(hold.settled)return;hold.settled=true;clearTimeout(timer);if(stream.rebinding===hold)stream.rebinding=null;settle(ok);}};
 host.timing?.('region_rebind_pending',performance.timeOrigin+performance.now());
 return hold;
}
// Row 487: swap a freshly admitted source/lease/window into the SAME stream (subscription, binding_id, sequence kept).
// The credit path sees the new source and restarts its encoder with a key frame at the new size; frames name the new
// binding sequence (window epoch) that input must use. The old source and lease close after the swap.
export async function rebindRegion(host,stream,{lease,args,window,tab,policy,generation,Capture}){
 let opened;
 try{opened=await openRegionSource({host,target:stream.regionTarget,lease,args,window,tab,policy,generation,Capture});}catch(error){await lease.close();throw error;}
 const hold=stream.rebinding;
 try{
  if(!await lease.isCurrent()||args.validateCurrent&&!await args.validateCurrent())refuse();
  // After the last await, nothing in between: the stream is still this subscription's, its hold still open (not timed out,
  // not unsubscribed), and the policy, generation and window unchanged (#1723 review).
  if(!hold||hold.settled||stream.rebinding!==hold||host.displays.get(stream.subscription_id)!==stream||host.protection!==policy||host.generation!==generation||!window.isCurrent())refuse();
 }catch(error){try{await opened.source.close();}finally{try{await opened.releaseRender?.();}finally{await lease.close();}}throw error;}
 const old={source:stream.regionSource,release:stream.regionRelease,lease:stream.regionLease};
 lease.binding_id=stream.binding_id;
 Object.assign(stream,{regionLease:lease,regionSource:opened.source,regionWindow:window,regionRelease:opened.releaseRender,
  css_width:opened.source.region.css_width,css_height:opened.source.region.css_height});
 const checks=regionChecks(host,stream,lease,window,opened);
 stream.regionCurrent=checks.regionCurrent;stream.regionInputCurrent=checks.regionInputCurrent;stream.regionFrameCurrent=checks.regionFrameCurrent;
 stream.invalidate(true,'rebind');
 host.timing?.('region_rebound',performance.timeOrigin+performance.now());
 try{await old.source.close();}finally{try{await old.release?.();}finally{await old.lease.close();}}
}
