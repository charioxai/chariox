// MP-08/MP-10/MP-11: kernel-owned visible-page admission over the shared region
// pump. No caller geometry, native handles, alternate ledger or input path.
import {randomUUID} from 'node:crypto';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
import {reserveDisplay} from './kernel-browser-display-admission.mjs';
import {createRegionDisplay} from './kernel-browser-region-display.mjs';
import {VisiblePageCapture,visiblePageGeometry} from './kernel-browser-visible-page.mjs';
const refuse=()=>{throw new UserDomainRefusal('stale_reference');};
export function subscribeVisiblePage(host,command,scope,signal,{Capture=VisiblePageCapture}={}){
 return reserveDisplay(host,async()=>{
  assertNotCancelled(signal);
  if(command.generation!==host.generation||command.target||
   !command.codecs?.includes('png')||!command.codecs.includes('avc1.420033')||command.codecs.length>8||
   !Number.isInteger(command.bitrate)||command.bitrate<500_000||command.bitrate>64_000_000||![1,2].includes(command.device_scale_factor))refuse();
  const tab=await host.displayTarget(command),policy=host.protection,generation=host.generation,document=tab.document_id;
  const window=host.chromeWindows.current().tabbed;
  const {connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id);
  const measure=async()=>{const m=await connection.send('Page.getLayoutMetrics',{},sessionId);return {clientWidth:m.cssLayoutViewport?.clientWidth,clientHeight:m.cssLayoutViewport?.clientHeight,zoom:m.cssVisualViewport?.zoom??1,scale:m.cssVisualViewport?.scale};};
  const viewport=await measure();
  const geometry=visiblePageGeometry(window,viewport);
  host.timing.event?.('native_visible_page_geometry',{client_width:viewport.clientWidth,client_height:viewport.clientHeight,crop_width:geometry.width,crop_height:geometry.height,scale:window.scale});
  const alive=()=>host.protection===policy&&!policy.unknown&&host.generation===generation&&window.isCurrent()&&host.tabs.get(tab.tab_id)?.document_id===document;
  const current=async()=>alive()&&typeof window.isPageCurrent==='function'&&await window.isPageCurrent()&&typeof host.arbiter?.isActiveTab==='function'&&await host.arbiter.isActiveTab(tab,window)&&JSON.stringify(await measure())===JSON.stringify(viewport)&&alive();
  if(!await current())refuse();
  const lease={binding_id:`host-page-region-${randomUUID()}`,document_id:document,sequence:window.window_generation,
   isAlive:alive,isCurrent:current,close:async()=>{}};
  return createRegionDisplay({host,command,scope,signal,target:{kind:'visible_page'},lease,
   args:{window,viewport,validateCurrent:current},window,tab,policy,generation,Capture});
 });
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
 if(!await stream.regionCurrent())refuse();
 return host.request({...page,observed_by:scope},{signal,inputGuard:async()=>{
  assertNotCancelled(signal);if(!await stream.regionCurrent())refuse();
 }});
}
