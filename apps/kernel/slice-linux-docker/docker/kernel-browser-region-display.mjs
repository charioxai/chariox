// MP-08/MP-10/MP-11: one region stream lifetime for page crops and Chrome UI.
import {randomUUID} from 'node:crypto';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
import {DisplayStream,PortableEncoder} from './kernel-browser-display.mjs';
const refuse=()=>{throw new UserDomainRefusal('stale_reference');};
const chromeUi=t=>['browser_chrome','browser_auxiliary'].includes(t?.kind);
export async function createRegionDisplay({host,command,scope,signal,target,lease,args,window,tab,policy,generation,placement,offset={x:0,y:0},overlayTab=null,Capture}){
 let source,stream;
 try {
  const pageTab=overlayTab??tab;
  const {connection,sessionId}=await host.browser.resolvePageTarget(pageTab.target_id);
  const scale=window.scale;
  // Browser UI survives page navigation; the frame document is its window epoch.
  // A dialog overlay's frames belong to the window epoch too; its page document fences it separately.
  const sourceTab=chromeUi(target)?{...pageTab,document_id:lease.document_id}:pageTab;
  source=new Capture({...args,...(overlayTab?{pageDocument:overlayTab.document_id}:{}),display:host.chromium.display,pid:window.pid??host.chromium.browserPid??host.chromium.child?.pid,connection,sessionId,tab:sourceTab,scale,
   hostScale:host.chromium.scale??1,policy,browser:host.browser,frames:id=>host.browser.frameSession?.(id,connection),
   screenshot:()=>host.displayScreenshot(pageTab,null,false),allowed:p=>host.protection===p&&!p.unknown&&host.generation===generation,timing:host.timing});
  // No scrim behind the "dialog" (a darker theme dimmed the toolbar): the kernel learns the toolbar again.
  try{await source.start();}catch(error){if(overlayTab&&error?.scrim!==undefined)host.chromeWindows.relearnToolbar?.();throw error;}
  assertNotCancelled(signal);
  if(host.protection!==policy||!await lease.isCurrent()||!window.isCurrent()||args.validateCurrent&&!await args.validateCurrent())refuse();
  const result={source_kind:'region',binding_id:lease.binding_id,document_id:lease.document_id,sequence:lease.sequence,
   generation,subscription_id:`host-display-${randomUUID()}`,codec:'avc1.420033',bitrate:command.bitrate,
   device_scale_factor:scale,css_width:source.region.css_width,css_height:source.region.css_height,
   ...(placement?{region_offset:placement}:{})};
  stream=new DisplayStream({...result,tab_id:tab.tab_id,observed_by:scope,relay_binary:command.codecs.includes('chariox-relay-binary-v96'),
   // Crops use whole-video dependencies; an eight-row encoder cannot divide
   // every valid even crop (including a two-device-pixel opaque tile).
   dependencies:command.codecs.includes('chariox-video-dependencies-v1'),stripes:false},
   {timing:host.timing,encoder:new PortableEncoder()});
  Object.assign(stream,{regionLease:lease,regionSource:source,regionTarget:target,regionWindow:window,regionOffset:offset,regionPlacement:placement,
   regionCurrent:args.validateCurrent??lease.isCurrent});
  const close=stream.close.bind(stream);let closing,offRetire;
  stream.close=()=>closing??=(async()=>{if(typeof offRetire==='function')offRetire();host.displays.delete(stream.subscription_id);try{await close();}finally{try{await source.close();}finally{await lease.close();}}})();
  if(target.kind==='region')lease.attachDisplay(stream);else host.displays.set(stream.subscription_id,stream);
  const unwatch=target.kind==='browser_chrome'&&typeof source.windows==='function'?host.chromeWindows.watch?.(()=>source.windows()):null;
  // The band's own frames tell when a browser-modal dialog dims the window (no other signal exists).
  const offBand=target.kind==='browser_chrome'&&typeof source.subscribe==='function'?source.subscribe(frame=>host.chromeWindows.observeBand?.(frame?.raw)):null;
  const offWindow=host.chromeWindows.onRetire?.(()=>{if(!window.isCurrent())void host.removeDisplay(stream.subscription_id).catch(()=>{});});
  offRetire=()=>{unwatch?.();offBand?.();offWindow?.();};
  host.armDisplayExpiry(stream);return result;
 }catch(error){if(stream)await stream.close();else{await source?.close();await lease.close();}throw error;}
}
