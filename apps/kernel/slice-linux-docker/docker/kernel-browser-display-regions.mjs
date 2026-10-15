// MP-08/MP-10/MP-11: public 519 adapter over the owners' private region leases.
// No client geometry, native handle, permission path, or separate frame pump.
import {randomUUID} from 'node:crypto';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {assertNotCancelled} from './browser-controller-actions.mjs';
import {NativeWindowRegionCapture} from './kernel-browser-window-region.mjs';
import {DisplayStream, PortableEncoder} from './kernel-browser-display.mjs';
import {inputHostTab} from './kernel-browser-input.mjs';

const refuse=()=>{throw new UserDomainRefusal('stale_reference');};
function checkedTarget(target) {
 const keys=target?.kind==='region'?['kind','mirror_subscription_id','document_id','sequence','region_id']:
  target?.kind==='browser_chrome'?['kind','window_generation']:[];
 if(!keys.length||Object.keys(target).some(key=>!keys.includes(key))||keys.some(key=>!(key in target)))refuse();
 return target;
}
export async function subscribeRegion(host,command,scope,signal,{Capture=NativeWindowRegionCapture}={}) {
 assertNotCancelled(signal);
 const target=checkedTarget(command.target);
 // State advertisement is not admission: a terminal can guess a window epoch.
 // Only managed lockdown proves Chrome cannot replace this unmasked band
 // with same-size fullscreen page pixels.
 if(target.kind==='browser_chrome'&&host.chromium?.policy!=='managed')refuse();
 if(command._agent_input||command.focused_agent||command.generation!==host.generation)refuse();
 if(!Array.isArray(command.codecs)||command.codecs.length>8||!command.codecs.includes('png')||!command.codecs.includes('avc1.420033')||
  !Number.isInteger(command.bitrate)||command.bitrate<500_000||command.bitrate>64_000_000||![1,2].includes(command.device_scale_factor)||host.displays.size>=8)refuse();
 const tab=await host.displayTarget(command),policy=host.protection,generation=host.generation;
 if(tab.tab_id!==command.tab_id)refuse();
 let lease,args,offset={x:0,y:0};
 const windows=host.chromeWindows?.current();
 const window=windows?.tabbed;
 if(!window||window.role!=='tabbed'||window.browser_generation!==generation||!window.isCurrent())refuse();
 if(target.kind==='region') {
  if(!host.mirror.v2?.regions)refuse();
  const {kind,...reference}=target;
  lease=await host.mirror.v2.regions.resolve({...reference,generation},scope,{signal});
  try {
   if(lease.tab_id!==tab.tab_id||lease.document_id!==tab.document_id||!await lease.isCurrent())refuse();
   const scale=lease.device_scale_factor,r=lease.rect;
   if(scale!==window.scale)refuse();
   // A hidden mirror can retain its document/geometry while another Tab paints
   // in this SAME X window. Only the browser owner's native active-tab observer
   // can admit that readback; focus-emulated visibility is not authority.
   if(typeof host.arbiter?.isActiveTab!=='function'||!await host.arbiter.isActiveTab(tab,window))refuse();
   // Stay INSIDE the canonical clip; the codec's even dimensions never expose
   // adjacent pixels. Keep the offset for the existing mirror input resolver.
   const left=Math.ceil(r.x*scale),top=Math.ceil(r.y*scale),width=2*Math.floor((Math.floor((r.x+r.width)*scale)-left)/2),height=2*Math.floor((Math.floor((r.y+r.height)*scale)-top)/2);
   offset={x:left/scale-r.x,y:top/scale-r.y};
   args={surface:'page',window:{id:window.xid,width:window.bounds.width,height:window.bounds.height},
    rect:{left,top:window.content_top+top,width,height},contentHeight:window.bounds.height-window.content_top,
    isCurrent:()=>lease.isAlive()&&window.isCurrent(),validateCurrent:async()=>window.isCurrent()&&await host.arbiter.isActiveTab(tab,window)&&await lease.isCurrent()};
  }catch(error){await lease.close();throw error;}
 } else {
  if(!Number.isSafeInteger(target.window_generation)||target.window_generation!==window.window_generation)refuse();
  const current=()=>host.chromium.policy==='managed'&&window.isCurrent();
  args={...host.chromeWindows.band(window),isCurrent:current};
  const binding_id=`host-chrome-region-${randomUUID()}`;
  lease={binding_id,generation,document_id:`chrome-window-${generation}-${window.window_generation}`,sequence:window.window_generation,
   isAlive:current,isCurrent:async()=>current(),close:async()=>{}};
 }
 let source,stream;
 try {
  const {connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id);
  const scale=window.scale;
  // Browser UI survives page navigation; the frame document is its window epoch.
  const sourceTab=target.kind==='browser_chrome'?{...tab,document_id:lease.document_id}:tab;
  source=new Capture({...args,display:host.chromium.display,pid:window.pid??host.chromium.browserPid??host.chromium.child?.pid,connection,sessionId,tab:sourceTab,scale,
   hostScale:host.chromium.scale??1,policy,browser:host.browser,frames:id=>host.browser.frameSession?.(id,connection),
   screenshot:()=>host.displayScreenshot(tab,null,false),allowed:p=>host.protection===p&&!p.unknown&&host.generation===generation,timing:host.timing});
  await source.start();assertNotCancelled(signal);
  if(host.protection!==policy||!await lease.isCurrent()||!window.isCurrent())refuse();
  const result={source_kind:'region',binding_id:lease.binding_id,document_id:lease.document_id,sequence:lease.sequence,
   generation,subscription_id:`host-display-${randomUUID()}`,codec:'avc1.420033',bitrate:command.bitrate,
   device_scale_factor:scale,css_width:source.region.css_width,css_height:source.region.css_height};
  stream=new DisplayStream({...result,tab_id:tab.tab_id,observed_by:scope,relay_binary:command.codecs.includes('chariox-relay-binary-v96'),
   // Crops use whole-video dependencies; an eight-row encoder cannot divide
   // every valid even crop (including a two-device-pixel opaque tile).
   dependencies:command.codecs.includes('chariox-video-dependencies-v1'),stripes:false},
   {timing:host.timing,encoder:new PortableEncoder()});
  Object.assign(stream,{regionLease:lease,regionSource:source,regionTarget:target,regionWindow:window,regionOffset:offset,
   regionCurrent:args.validateCurrent??lease.isCurrent});
  const close=stream.close.bind(stream);let closing,offRetire;
  stream.close=()=>closing??=(async()=>{if(typeof offRetire==='function')offRetire();host.displays.delete(stream.subscription_id);try{await close();}finally{try{await source.close();}finally{await lease.close();}}})();
  if(target.kind==='region')lease.attachDisplay(stream);else host.displays.set(stream.subscription_id,stream);
  offRetire=host.chromeWindows.onRetire?.(()=>{if(!window.isCurrent())void host.removeDisplay(stream.subscription_id).catch(()=>{});});
  host.armDisplayExpiry(stream);return result;
 }catch(error){if(stream)await stream.close();else{await source?.close();await lease.close();}throw error;}
}
function inputStream(host,command,scope) {
 if(command._agent_input||command.focused_agent||command.generation!==host.generation)refuse();
 const stream=[...host.displays.values()].find(s=>s.binding_id===command.binding_id);
 if(!stream||stream.observed_by!==scope||
  (stream.tab_id!==command.tab_id&&!(stream.regionTarget.kind==='browser_chrome'&&command.region_document_id&&host.tabs.has(command.tab_id)))||stream.expires<=Date.now()||
  stream.regionLease.document_id!==(command.region_document_id??command.document_id)||stream.regionLease.sequence!==command.sequence||
  !stream.regionLease.isAlive()||!stream.regionWindow.isCurrent())refuse();
 return stream;
}
// Read-only pre-ledger adaptation: the owner arbiter observes the ACTIVE page.
// The ordinary Rust actor ledger still admits every resulting input action.
export async function regionInputTarget(host,command,scope,signal) {
 const stream=inputStream(host,command,scope);assertNotCancelled(signal);
 if(!await stream.regionCurrent())refuse();
 if(stream.regionTarget.kind==='region')return {tab_id:stream.tab_id,document_id:stream.regionLease.document_id};
 if(typeof host.arbiter?.resolveDisplayInput!=='function')refuse();
 // This window epoch is kernel-issued private adapter data, not an extra
 // public DisplayInput field or a client window claim.
 const target=await host.arbiter.resolveDisplayInput({...command,window_generation:stream.regionWindow.window_generation},scope,{signal,stream});
 if(!target||!host.tabs.has(target.tab_id)||!target.document_id)refuse();
 return {tab_id:target.tab_id,document_id:target.document_id,region_document_id:stream.regionLease.document_id};
}
export async function regionInput(host,command,scope,signal) {
 const stream=inputStream(host,command,scope);assertNotCancelled(signal);
 if(!await stream.regionCurrent())refuse();
 if(stream.regionTarget.kind==='browser_chrome') {
  if(typeof host.arbiter?.input!=='function')refuse();
  return host.arbiter.input({stream,command,scope,signal});
 }
 const input=command.input;
 if(!['click','scroll'].includes(input?.kind)||!Number.isInteger(input.x)||!Number.isInteger(input.y)||input.x<0||input.y<0||input.x>=stream.css_width||input.y>=stream.css_height)refuse();
 const resolved=await stream.regionLease.resolveInput({...input,sequence:command.sequence,x:input.x+stream.regionOffset.x,y:input.y+stream.regionOffset.y},scope,{signal});
 const guard=resolved.guard;
 resolved.guard=async()=>{if(!await stream.regionCurrent())refuse();await guard?.();};
 if(host.dispatchRegionInput)return host.dispatchRegionInput(stream,resolved,command,signal);
 const tab=await host.displayTarget(command);
 if(tab.document_id!==stream.regionLease.document_id)refuse();
 let dispatched=false;
 try { await host.sampleLane(tab).run('input',()=>host.popupEvidence.capture(host.browser,tab,command._action_id,beginDispatch=>
  inputHostTab(host.browser,tab,{kind:'mirror'},{signal,resolveMirror:async()=>resolved,onDispatch:()=>{
   dispatched=true;
   host.inputEpochs.set(tab.tab_id,(host.inputEpochs.get(tab.tab_id)??0)+1);host.inputChangedAt.set(tab.tab_id,performance.now());stream.regionSource.wake();return beginDispatch();
  }}))); }
 catch(error){if(dispatched||['browser_action_cancelled','stale_document_reference'].includes(error?.code))await host.stop();throw error;}
 return host.observe(await host.reconcile(),null,scope);
}
