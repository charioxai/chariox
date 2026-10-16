// MP-08/MP-10/MP-11: kernel-owned visible-page admission over the shared region
// pump. No caller geometry, native handles, alternate ledger or input path.
import {randomUUID} from 'node:crypto';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
import {reserveDisplay} from './kernel-browser-display-admission.mjs';
import {createRegionDisplay} from './kernel-browser-region-display.mjs';
import {VisiblePageCapture,visiblePageGeometry} from './kernel-browser-visible-page.mjs';
const refuse=()=>{throw new UserDomainRefusal('stale_reference');};
// MP-08/MP-10/MP-11: one shared human-viewer predicate, also used before
// selecting cached document metadata. It grants no input by itself.
export function viewerPageInputActive(host,command,scope){
 return command._display_input===true&&!command._agent_input&&!command.focused_agent&&
  [...host.displays.values()].some(s=>s.tab_id===command.tab_id&&s.observed_by===scope&&s.expires>Date.now());
}
export function nativeKeyDocumentTarget(host,command,scope,nativeDisplay){
 if(command.op!=='input'||command.input?.kind!=='key'||!viewerPageInputActive(host,command,scope))return false;
 const source=nativeDisplay?.regionSource??host.compositors.get(command.tab_id)?.source;
 return source?.native===true&&source.attested===true&&source.valid?.()===true;
}
export function subscribeVisiblePage(host,command,scope,signal,{Capture=VisiblePageCapture}={}){
 return reserveDisplay(host,async reservation=>{
  assertNotCancelled(signal);
  if(command.generation!==host.generation||command.target||
   !command.codecs?.includes('png')||!command.codecs.includes('avc1.420033')||command.codecs.length>8||
   !Number.isInteger(command.bitrate)||command.bitrate<500_000||command.bitrate>64_000_000||![1,2].includes(command.device_scale_factor))refuse();
  // MP-08/MP-10/MP-11: the mirror and its native preview use the same
  // event-bound document owner. Stored state may lag a committed navigation.
  const tab=await (host.mirrorTarget??host.displayTarget).call(host,command),policy=host.protection,generation=host.generation,document=tab.document_id;
  reservation.reference={kind:'visible_page',tab_id:tab.tab_id,document_id:document,generation,scope};
  const window=host.chromeWindows.current().tabbed;
  const frameProof=host.browser.currentPageFrame?.(tab.target_id);
  const {connection,sessionId}=frameProof??await host.browser.resolvePageTarget(tab.target_id);
  const documentCurrent=()=>host.tabs.get(tab.tab_id)?.target_id===tab.target_id&&(frameProof
   ?host.browser.currentPageFrame?.(tab.target_id)===frameProof&&frameProof.frame.loaderId===document
   :host.tabs.get(tab.tab_id)?.document_id===document);
  const measure=async()=>{const m=await connection.send('Page.getLayoutMetrics',{},sessionId);return {clientWidth:m.cssLayoutViewport?.clientWidth,clientHeight:m.cssLayoutViewport?.clientHeight,zoom:m.cssVisualViewport?.zoom??1,scale:m.cssVisualViewport?.scale};};
  const browser=host.browser,metrics=browser.viewportByTarget?.get(tab.target_id);
  // The existing native observer retires this measurement on resize, zoom,
  // tab/window changes or detach. A cold binding retains fresh measurement.
  const viewportGuard=frameProof&&typeof metrics==='string'?host.tabGroups?.nativeStateGuard?.():null;
  const viewport=await measure();
  const geometry=visiblePageGeometry(window,viewport);
  host.timing.event?.('native_visible_page_geometry',{client_width:viewport.clientWidth,client_height:viewport.clientHeight,crop_width:geometry.width,crop_height:geometry.height,scale:window.scale});
  const alive=()=>host.browser===browser&&host.protection===policy&&!policy.unknown&&host.generation===generation&&window.isCurrent()&&documentCurrent()&&(!viewportGuard||viewportGuard()&&browser.viewportByTarget?.get(tab.target_id)===metrics);
  const current=async()=>alive()&&typeof window.isPageCurrent==='function'&&await window.isPageCurrent(tab.target_id)&&(viewportGuard?viewportGuard():JSON.stringify(await measure())===JSON.stringify(viewport))&&alive();
  if(!await current())refuse();
  const lease={binding_id:`host-page-region-${randomUUID()}`,document_id:document,sequence:window.window_generation,
   isAlive:alive,isCurrent:current,close:async()=>{}};
  return createRegionDisplay({host,command,scope,signal,target:{kind:'visible_page'},lease,
   args:{window,viewport,validateCurrent:current,cachedCurrent:typeof window.isPageCurrentCached==='function'?()=>alive()&&window.isPageCurrentCached(tab.target_id):undefined,occluded:async()=>{
    if(typeof host.chromeWindows.inWindowOverNow!=='function')throw Error('MP-11: visible page occlusion unavailable');
    return host.chromeWindows.inWindowOverNow({left:geometry.window_region.left,top:geometry.window_region.top,width:geometry.width,height:geometry.height});
   }},window,tab,policy,generation,Capture});

 },{kind:'visible_page',tab_id:command.tab_id,generation:command.generation,scope});
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
  if(!await stream.regionCurrent())refuse();
  visiblePageInputSource(host,stream,tab,scope);
 };
 await inputGuard();
 return host.request({...page,observed_by:scope},{signal,inputGuard,nativeDisplay:stream});
}

// MP-08/MP-11: the private frame-size hint may select an existing kernel crop;
// it never grants a caller rectangle, a different observer or a stale document.
export async function captureVisiblePage(host,tab,frame,scope){
 const stream=[...host.displays.values()].find(s=>s.regionTarget?.kind==='visible_page'&&s.observed_by===scope&&
  s.tab_id===tab.tab_id&&s.generation===host.generation&&s.regionLease.document_id===tab.document_id&&
  s.css_width*s.device_scale_factor===frame?.width&&s.css_height*s.device_scale_factor===frame?.height);
 if(!stream)return captureMirroredPage(host,tab,frame,scope);
 const policy=host.protection;
 const alive=()=>host.protection===policy&&!policy.unknown&&stream&&!stream.suspended&&host.displays.get(stream.subscription_id)===stream&&
  stream.regionLease.isAlive()&&stream.regionSource.valid()&&stream.regionWindow.isCurrent();
 const current=async()=>alive()&&await stream.regionCurrent()&&alive();
 if(!await current())refuse();
 const shot=await host.displayScreenshot(tab,{x:0,y:0,width:stream.css_width,height:stream.css_height,scale:1},false);
 if(!await current()||shot.width!==frame.width||shot.height!==frame.height)refuse();
 return {...shot,protected_regions:shot[displayMaskRegions]??[]};
}

// MP-08/MP-10/MP-11: handover closes the temporary video producer. A current
// opted-in mirror retains observation authority, never its old pixels/geometry.
// Resolve the owned crop again and use the ordinary protected screenshot seam.
async function captureMirroredPage(host,tab,frame,scope){
 const mirror=[...(host.mirror?.streams?.values()??[])].find(s=>s.wire===2&&s.navigationPreview&&s.scope===scope&&s.tab_id===tab.tab_id&&s.document_id===tab.document_id);
 if(!mirror||host.chromium?.policy!=='managed')refuse();
 const policy=host.protection,generation=host.generation,document=tab.document_id,window=host.chromeWindows.current().tabbed;
 const alive=()=>host.protection===policy&&!policy.unknown&&host.generation===generation&&window.isCurrent()&&
  mirror.policy===policy&&mirror.expires>Date.now()&&[...host.mirror.streams.values()].includes(mirror)&&host.tabs.get(tab.tab_id)?.document_id===document;
 if(!alive()||window.scale!==host.scales.get(tab.tab_id))refuse();
 const {connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id);
 const measure=async()=>{const m=await connection.send('Page.getLayoutMetrics',{},sessionId);return {clientWidth:m.cssLayoutViewport?.clientWidth,clientHeight:m.cssLayoutViewport?.clientHeight,zoom:m.cssVisualViewport?.zoom??1,scale:m.cssVisualViewport?.scale};};
 const viewport=await measure(),geometry=visiblePageGeometry(window,viewport);
 if(geometry.width!==frame?.width||geometry.height!==frame?.height)refuse();
 const current=async()=>alive()&&await window.isPageCurrent(tab.target_id)&&JSON.stringify(await measure())===JSON.stringify(viewport)&&alive();
 if(!await current())refuse();
 const shot=await host.displayScreenshot(tab,{x:0,y:0,width:geometry.css_width,height:geometry.css_height,scale:1},false);
 if(!await current()||shot.width!==frame.width||shot.height!==frame.height)refuse();
 return {...shot,protected_regions:shot[displayMaskRegions]??[]};
}
