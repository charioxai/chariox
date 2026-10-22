// MP-08/MP-10/MP-11: kernel-owned visible-page admission over the shared region
// pump. No caller geometry, native handles, alternate ledger or input path.
import {randomUUID} from 'node:crypto';
import {CaptureRefusal,UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
import {displayGeometry} from './kernel-browser-geometry.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
import {reserveDisplay} from './kernel-browser-display-admission.mjs';
import {createRegionDisplay} from './kernel-browser-region-display.mjs';
import {VisiblePageCapture,visiblePageGeometry} from './kernel-browser-visible-page.mjs';
import {nativeRefusalReason} from './kernel-browser-native.mjs';
import {observeLayout} from './kernel-browser-layout-observer.mjs';
import {beginNavigationPreview} from './kernel-browser-mirror2-preview.mjs';
const refuse=()=>{throw new UserDomainRefusal('stale_reference','region_retired');};
// MP-08/MP-10/MP-11: one shared human-viewer predicate, also used before
// selecting cached document metadata. It grants no input by itself.
export function viewerPageInputActive(host,command,scope){
 return command._display_input===true&&!command._agent_input&&!command.focused_agent&&
  [...host.displays.values()].some(s=>s.tab_id===command.tab_id&&s.observed_by===scope&&s.expires>Date.now());
}
export function nativeInputDocumentTarget(host,command,scope,nativeDisplay){
 if(command.op!=='input'||!['key','scroll'].includes(command.input?.kind)||!viewerPageInputActive(host,command,scope))return false;
 const source=nativeDisplay?.regionSource??host.compositors.get(command.tab_id)?.source;
 return source?.native===true&&source.attested===true&&source.valid?.()===true;
}
// MP-08/MP-10/MP-11: a renderer parsing a huge document answers CDP late (r47 WHATWG: every preview attach hit the
// 5 s timeout). The layout read only narrows the crop inside the native page rect, so this target's last read under
// the same emulated metrics and window binding stands in while the renderer is busy; a later read that differs
// fails the current check and retires the display.
// r41: a native start that cannot run (no pixel match: an animating page while a Vault fill is tracked; no
// readback) serves this subscription from the CDP display (protected PNG captures, no native window) instead
// of leaving the viewer on stale pixels; the next document subscribes to native again. Stale-document and
// lease refusals still propagate: the viewer retries those against the new document.
const pngFallbackReasons=new Set(['attestation_mismatch','first_frame_timeout']);
export function pngFallbackCommand(error,command){
 if(!pngFallbackReasons.has(nativeRefusalReason(error)))return null;
 return {...command,codecs:command.codecs.filter(c=>c==='png'||c.startsWith('chariox-'))};
}
const LAYOUT_WAIT_MS=400,layouts=new WeakMap(),proofs=new WeakMap();
export function subscribeVisiblePage(host,command,scope,signal,{Capture=VisiblePageCapture}={}){
 let complete;
 return reserveDisplay(host,async()=>{
  assertNotCancelled(signal);
  if(command.generation!==host.generation||command.target||
   !command.codecs?.includes('png')||!command.codecs.includes('avc1.420033')||command.codecs.length>8||
   !Number.isInteger(command.bitrate)||command.bitrate<500_000||command.bitrate>64_000_000||![1,2].includes(command.device_scale_factor))refuse();
  // MP-08/MP-10/MP-11: the mirror and its native preview use the same
  // event-bound document owner. Stored state may lag a committed navigation.
  const tab=await (host.mirrorTarget??host.displayTarget).call(host,command),policy=host.protection,generation=host.generation,document=tab.document_id;
  complete=beginNavigationPreview(host,{tab_id:tab.tab_id,document_id:document,generation,scope});
  const window=host.chromeWindows.current().tabbed;
  const frameProof=host.browser.currentPageFrame?.(tab.target_id);
  const {connection,sessionId}=frameProof??await host.browser.resolvePageTarget(tab.target_id);
  const documentCurrent=()=>host.tabs.get(tab.tab_id)?.target_id===tab.target_id&&(frameProof
   ?host.browser.currentPageFrame?.(tab.target_id)===frameProof&&frameProof.frame.loaderId===document
   :host.tabs.get(tab.tab_id)?.document_id===document);
  const browser=host.browser,metrics=browser.viewportByTarget?.get(tab.target_id);
  const reads=layouts.get(browser)??new Map(),key=JSON.stringify([metrics??null,window.window_generation,window.page_rect]);layouts.set(browser,reads);
  // A crop served from the last read is retired when the renderer's delayed read disagrees.
  // A renderer that is late is not a renderer that disagrees: a read that timed out keeps the crop and is read again,
  // a few times quickly and then slowly (each read makes a loading page lay out), while the display lives; any other failure retires it.
  let differs=false,closed=false;const watched=new WeakSet(),verify=(used,fresh,retries=3)=>{if(watched.has(fresh))return used;watched.add(fresh);fresh.then(viewport=>{if(JSON.stringify(viewport)!==JSON.stringify(used))differs=true;},error=>{
   if(differs)return;if(error?.code==='browser_cdp_timeout'){{const wait=host.layoutRetryMs??1000;setTimeout(()=>{if(!closed&&alive())verify(used,read(),retries-1)},retries>0?wait:wait*5);}}else differs=true;});return used;};
  const read=()=>connection.send('Page.getLayoutMetrics',{},sessionId).then(m=>{const viewport={clientWidth:m.cssLayoutViewport?.clientWidth,clientHeight:m.cssLayoutViewport?.clientHeight,zoom:m.cssVisualViewport?.zoom??1,scale:m.cssVisualViewport?.scale};reads.set(tab.target_id,{key,viewport,pending:null});return viewport;});
  const measure=async()=>{
   const last=reads.get(tab.target_id),cached=last?.key===key?last:null;
   if(cached?.pending)return verify(cached.viewport,cached.pending);
   const fresh=read();
   if(!cached)return fresh;
   cached.pending=fresh;fresh.catch(()=>{if(cached.pending===fresh)cached.pending=null;});let timer;
   try{return await Promise.race([fresh,new Promise(resolve=>{timer=setTimeout(()=>resolve(verify(cached.viewport,fresh)),LAYOUT_WAIT_MS);})]);}finally{clearTimeout(timer);}
  };
  // The existing native observer retires this measurement on resize, zoom,
  // tab/window changes or detach. A cold binding retains fresh measurement.
  const viewportGuard=frameProof&&typeof metrics==='string'?host.tabGroups?.nativeStateGuard?.():null;
  const viewport=await measure();
  const geometry=visiblePageGeometry(window,viewport);
  host.timing.event?.('native_visible_page_geometry',{client_width:viewport.clientWidth,client_height:viewport.clientHeight,crop_width:geometry.width,crop_height:geometry.height,scale:window.scale});
  // Diagnostics only: the first condition that retired this binding, named once.
  let named=false;
  const why=()=>differs?'layout_differs':host.browser!==browser?'browser':host.protection!==policy||policy.unknown?'protection':host.generation!==generation?'generation':!window.isCurrent()?'window':!documentCurrent()?(host.tabs.get(tab.tab_id)?.target_id!==tab.target_id?'tab':frameProof?(host.browser.currentPageFrame?.(tab.target_id)!==frameProof?'frame_record':'loader'):'document_id'):viewportGuard&&!viewportGuard()?`guard ${host.tabGroups?.nativeState?.cause??''}`:'metrics';
  const name=()=>{if(!named){named=true;host.timing?.(`native_lease_dead ${why()}`,performance.timeOrigin+performance.now());}return false;};
  const aliveNow=()=>!differs&&host.browser===browser&&host.protection===policy&&!policy.unknown&&host.generation===generation&&window.isCurrent()&&documentCurrent()&&(!viewportGuard||viewportGuard()&&browser.viewportByTarget?.get(tab.target_id)===metrics);
  const alive=()=>aliveNow()||name();
  const current=async()=>alive()&&typeof window.isPageCurrent==='function'&&await window.isPageCurrent(tab.target_id)&&(viewportGuard?viewportGuard():JSON.stringify(await measure())===JSON.stringify(viewport))&&alive();
  if(!await current())refuse();
  // Publication reuses the layout read while a page-side observer proves it current. A native-state guard already answers
  // without a read; the observer installs in the background so a busy renderer delays nothing.
  let observer=null,observerClosed=false;
  const closeObserver=()=>{observerClosed=true;observer?.close();observer=null;};
  const layoutRead=async()=>JSON.stringify(await measure())===JSON.stringify(viewport);
  const publishLayout=()=>viewportGuard?viewportGuard():observer?observer.valid()||observer.confirm(layoutRead):layoutRead();
  // A refused publication fences the source (a new attach follows): the mark names the check that refused.
  const publishable=async()=>{
   const refused=reason=>{host.timing?.(`native_publish_refused ${reason}`,performance.timeOrigin+performance.now());return false;};
   if(!alive())return refused('not_alive');
   if(typeof window.isPageCurrent!=='function'||!await (window.isPagePublishable??window.isPageCurrent)(tab.target_id))return refused('page');
   if(!await publishLayout())return refused('layout');
   return alive()||refused('not_alive_after');
  };
  // MP-08/MP-11: exact native attestation of this binding on this connection admits a later document of it.
  // The crop is not part of it: a new document without a scrollbar widens the crop inside the same page rect.
  const proofKey=JSON.stringify([metrics??null,window.window_generation,window.page_rect,window.scale,host.chromium.scale??1]);
  const attestation={proven:()=>proofs.get(connection)?.get(tab.target_id)===proofKey,
   prove:()=>{const proven=proofs.get(connection)??new Map();proven.set(tab.target_id,proofKey);proofs.set(connection,proven);}};
  const lease={binding_id:`host-page-region-${randomUUID()}`,document_id:document,sequence:window.window_generation,
   isAlive:alive,isCurrent:current,close:async()=>{closed=true;closeObserver();}};
  if(!viewportGuard)observeLayout(connection,sessionId).then(o=>{if(observerClosed)o?.close();else observer=o;});
  try{return await createRegionDisplay({host,command,scope,signal,target:{kind:'visible_page'},lease,
   args:{window,viewport,attestation,validateCurrent:current,publishCurrent:publishable,cachedCurrent:typeof window.isPageCurrentCached==='function'?()=>alive()&&window.isPageCurrentCached(tab.target_id):undefined,occluded:async()=>{
    if(typeof host.chromeWindows.inWindowOverNow!=='function')throw Error('MP-11: visible page occlusion unavailable');
    return host.chromeWindows.inWindowOverNow({left:geometry.window_region.left,top:geometry.window_region.top,width:geometry.width,height:geometry.height});
   }},window,tab,policy,generation,Capture});}catch(error){closeObserver();throw error;}

 }).then(result=>{complete?.(true);return result;},error=>{complete?.(false);throw error;});
}
// MP-08/MP-10/MP-11: a private source hint can reuse only this exact live
// crop. It grants no input authority; ordinary admission and the final guard run.
export function visiblePageInputSource(host,stream,tab,scope){
 if(host.displays.get(stream.subscription_id)!==stream||stream.regionTarget?.kind!=='visible_page'||
  stream.tab_id!==tab.tab_id||stream.generation!==host.generation||stream.regionLease?.document_id!==tab.document_id||
  stream.observed_by!==scope||stream.suspended||!(stream.expires>Date.now())||
  !stream.regionWindow?.isCurrent()||!stream.regionSource?.attested||!stream.regionSource.valid())refuse();
 return stream.regionSource;
}
export async function inputVisiblePage(host,stream,command,scope,signal){
 // The caller already re-admitted this exact source/window/document. Keep all
 // normal page target, focus, cancellation and actor checks in host.request.
 const input=command.input;
 if(!['click','drag','scroll','key','text'].includes(input?.kind))refuse();
 for(const [x,y]of input.kind==='drag'?[[input.x,input.y],[input.to_x,input.to_y]]:
  ['click','scroll'].includes(input.kind)?[[input.x,input.y]]:[])
  if(!Number.isSafeInteger(x)||!Number.isSafeInteger(y)||x<0||y<0||x>=stream.css_width||y>=stream.css_height)refuse();
 const {binding_id,sequence,region_document_id,...page}=command;
 assertNotCancelled(signal);
 const inputGuard=async()=>{
  assertNotCancelled(signal);
  const tab=host.tabs.get(command.tab_id);if(!tab)refuse();
  visiblePageInputSource(host,stream,tab,scope);
  if(!await (stream.regionInputCurrent??stream.regionCurrent)(input))refuse();
  visiblePageInputSource(host,stream,tab,scope);
 };
 // The live crop check runs as a fence before the dispatch, where a busy renderer parks the input.
 const tab=host.tabs.get(command.tab_id);if(!tab)refuse();visiblePageInputSource(host,stream,tab,scope);
 return host.request({...page,observed_by:scope},{signal,inputGuard,nativeDisplay:stream});
}

// MP-08/MP-11: the owner's selection is served by the ordinary protected screenshot (exact Vault values masked,
// document-fenced) of the top-left css area the viewer's frame covers, at the Tab's emulated scale. The frame size is a
// hint, never a caller rectangle: it must fit inside the kernel viewport. No live stream, mirror or window read binds
// the capture, so it works right after a navigation.
export async function captureVisiblePage(host,tab,frame){
 const scale=host.scales.get(tab.tab_id)??host.viewport().device_scale_factor,width=frame?.width/scale,height=frame?.height/scale;
 if(!Number.isInteger(width)||!Number.isInteger(height)||width<1||height<1||width>displayGeometry.width||height>displayGeometry.height)throw new CaptureRefusal('view_resized');
 const shot=await host.displayScreenshot(tab,{x:0,y:0,width,height,scale:1},false);
 if(shot.width!==frame.width||shot.height!==frame.height)throw new CaptureRefusal('view_resized');
 return {...shot,protected_regions:shot[displayMaskRegions]??[]};
}
