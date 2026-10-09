import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
import {maskPng,opaqueFrame,cropProtectedPng,displayMaskRegions,displayFullMaskRegions} from './kernel-browser-pixels.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';
import {measurePageProtection,protectionDigest} from './browser-protection-regions.mjs';
// Screenshot-region masks from trusted CDP metadata, never page JavaScript.
const explicitMarkers=['data-chariox-secret','data-chariox-observation-protected','data-observation-protected'];
const protectionAttributes=new Set(['type','autocomplete',...explicitMarkers]);
// MP-11: protected node kinds. Frames and form fields that have no box render
// no pixels; explicit markers and opaque shadow hosts without a box (e.g.
// display:contents) cannot be bounded and mask the whole viewport.
function protectedKind(node,attrs){
  if(explicitMarkers.some(key=>attrs.has(key)))return 'explicit';
  if(node.localName==='input'&&(attrs.get('type')?.toLowerCase()==='password'||/password|one-time-code|cc-/i.test(attrs.get('autocomplete')??'')))return 'input';
  if(node.localName==='iframe'||node.localName==='frame')return 'frame';
  return null;
}
const attributesOf=node=>{const attrs=new Map();for(let i=0;i<(node.attributes?.length??0);i+=2)attrs.set(node.attributes[i],node.attributes[i+1]);return attrs;};
// MP-08/MP-11: one trusted DOM inspection. Open shadow roots are inspected
// through CDP (their protected descendants are masked, the host is cleared);
// closed or unknown roots keep their host opaque. Over-limit trees degrade to
// a whole-viewport mask instead of refusing capture for the session.
// MP-11 (owner 2026-10-08): every frame is inspected, not masked. In-process
// documents arrive with the pierced document (boxes in its viewport); isolated
// frames use the browser's auto-attached flat sessions (frames(frameId)).
// A frame is masked whole only under a protection marker on its owner or an
// ancestor, or when it cannot be inspected; record(reason) names why.
async function inspect(connection, sessionId, { frames = () => null, record = () => {}, sessions: watched = new Set() } = {}) {
  const nodes=[], sessions=new Set(), documents=[{sessionId,frame:null}];
  let visited=0;
  while (documents.length) {
    const {sessionId:session,frame}=documents.shift(), at={sessionId:session,frame};
    let root, nodeIds;
    watched.add(session);sessions.add(session);
    try {
      ({ root } = await connection.send("DOM.getDocument", { depth: -1, pierce: true }, session));
      ({ nodeIds } = await connection.send("DOM.querySelectorAll", { nodeId: root.nodeId, selector }, session));
      if (!Array.isArray(nodeIds)) throw new Error("Capture protection unavailable");
    } catch (error) { if (!frame) throw error; nodes.push({ nodeId: frame.nodeId, backendNodeId: frame.backendNodeId, kind: 'frame', ...frame.owner }); record('frame_uninspectable'); continue; }
    // MP-11: measure by backend node id. Another inspector's getDocument on
    // this session (e.g. a protected CDP capture) resets session node ids.
    const kinds=new Map(), seen=new Set(), backend=new Map(), pending=[[root,false]];
    while (pending.length) {
      if (++visited > 100_000) return {nodes:[],unbounded:true,sessions};
      const [node,marked] = pending.pop();
      if(Number.isSafeInteger(node.backendNodeId))backend.set(node.nodeId,node.backendNodeId);
      for(const shadow of node.shadowRoots??[]){
        if(shadow.shadowRootType==='user-agent')continue;
        if(shadow.shadowRootType==='open')pending.push([shadow,marked]);else kinds.set(node.nodeId,'opaque');
      }
      const kind=node.localName?protectedKind(node,attributesOf(node)):null;
      if(kind==='frame'){
        seen.add(node.nodeId);
        const child=!marked&&!node.contentDocument&&typeof node.frameId==='string'&&frames(node.frameId);
        if(marked||!node.contentDocument&&!child){kinds.set(node.nodeId,'frame');record(marked?'frame_marked':'frame_session_unavailable');}
        else if(child)documents.push({sessionId:child,frame:{nodeId:node.nodeId,backendNodeId:backend.get(node.nodeId),owner:at}});
        else pending.push([node.contentDocument,false]);
        continue;
      }
      if(kind)kinds.set(node.nodeId,kinds.get(node.nodeId)==='opaque'?'opaque':kind);
      for(const child of node.children??[])pending.push([child,marked||kind==='explicit']);
    }
    // A selector match the walk did not classify stays conservatively unbounded.
    for(const id of nodeIds)if(!kinds.has(id)&&!seen.has(id))kinds.set(id,'explicit');
    for(const [nodeId,kind] of [...kinds].sort((a,b)=>a[0]-b[0]))nodes.push({nodeId,backendNodeId:backend.get(nodeId),kind,...at});
  }
  if (nodes.length > 1024) return {nodes:[],unbounded:true,sessions};
  return {nodes,unbounded:false,sessions};
}
const selector='input[type="password"], [data-chariox-secret], [data-chariox-observation-protected], [data-observation-protected], input[autocomplete*="password" i], input[autocomplete*="one-time-code" i], input[autocomplete*="cc-" i], iframe, frame';
// A backend node that no longer exists renders nothing; session node ids are
// only a fallback; a stale session id fails closed (full mask, re-inspection).
const notRendered=/Could not compute box model|No node found for given backend id/;
const target=node=>Number.isSafeInteger(node.backendNodeId)?{backendNodeId:node.backendNodeId}:{nodeId:node.nodeId};
const rectOf=quad=>{
  if (!Array.isArray(quad) || quad.length !== 8 || quad.some(n => !Number.isFinite(n))) return null;
  const xs = [quad[0], quad[2], quad[4], quad[6]], ys = [quad[1], quad[3], quad[5], quad[7]];
  return {left:Math.min(...xs),top:Math.min(...ys),right:Math.max(...xs),bottom:Math.max(...ys)};
};
// Viewport rect of an isolated frame's content in its top document. Its own
// boxes are local; a transformed owner (scale/rotate) is bounded by its border.
async function frameRect(connection,frame,cache){
  const key=frame.owner.sessionId+' '+frame.nodeId;
  if(!cache.has(key))cache.set(key,(async()=>{
    let model;
    try{model=(await connection.send("DOM.getBoxModel",target(frame),frame.owner.sessionId)).model;}
    catch(error){if(!notRendered.test(error?.message??''))throw error;return undefined;}
    const border=rectOf(model?.border),content=rectOf(model?.content);
    if(!border||!content)return null;
    const parent=frame.owner.frame?await frameRect(connection,frame.owner.frame,cache):{left:0,top:0,right:geometry.width,bottom:geometry.height,origin:[0,0]};
    if(!parent||parent.whole)return parent;
    const [x1,y1,x2,y2,,,x4,y4]=model.border,near=(a,b)=>Math.abs(a-b)<0.5;
    const plain=near(border.right-border.left,model.width)&&near(border.bottom-border.top,model.height)&&near(y1,y2)&&near(x1,x4)&&x1<x2&&y1<y4;
    const box=plain?content:border,[x,y]=parent.origin;
    return {left:Math.max(parent.left,box.left+x),top:Math.max(parent.top,box.top+y),right:Math.min(parent.right,box.right+x),bottom:Math.min(parent.bottom,box.bottom+y),origin:[content.left+x,content.top+y],whole:!plain};
  })());
  return cache.get(key);
}
// Current viewport bounds of an inspection; null when any node is unbounded.
// Frames and form fields without a box (hidden, detached) render nothing.
async function measure(connection, inspection) {
  if(inspection.unbounded)return null;
  const cache=new Map();
  const rects=await Promise.all(inspection.nodes.map(async node=>{const {kind,sessionId,frame}=node;
    const clip=frame?await frameRect(connection,frame,cache):{left:0,top:0,right:geometry.width,bottom:geometry.height,origin:[0,0]};
    if(!clip)return clip;
    let box;
    if(clip.whole)box=clip;
    else{
      let quad;
      try{quad=(await connection.send("DOM.getBoxModel", target(node), sessionId)).model?.border;}
      catch(error){if(!notRendered.test(error?.message??''))throw error;return kind==='frame'||kind==='input'?undefined:null;}
      const rect=rectOf(quad);if(!rect)return null;
      box={left:rect.left+clip.origin[0],top:rect.top+clip.origin[1],right:rect.right+clip.origin[0],bottom:rect.bottom+clip.origin[1]};
    }
    // Only on-screen pixels can leak; an empty clip is no region.
    const left=Math.max(clip.left,box.left),top=Math.max(clip.top,box.top),right=Math.min(clip.right,box.right),bottom=Math.min(clip.bottom,box.bottom);
    return right>left&&bottom>top?{ x:left, y:top, width:right-left, height:bottom-top }:undefined;
  }));
  return rects.includes(null)?null:rects.filter(Boolean);
}
async function regions(connection, sessionId, options) {
  return await measure(connection,await inspect(connection,sessionId,options))??[{x:0,y:0,width:geometry.width,height:geometry.height}];
}

export const protectedHostRegions = (connection, sessionId, options) => regions(connection, sessionId, options);

// MP-11: page protection can change without repainting any pixel. Legacy
// callers pass hasRegions; a native tracker narrows the events to those that
// can add a protected node (bounds of tracked nodes are re-measured per frame).
const structural=['DOM.documentUpdated','DOM.childNodeInserted','DOM.childNodeRemoved','DOM.childNodeCountUpdated','DOM.shadowRootPushed','DOM.shadowRootPopped'];
const mayProtect=node=>!node||Boolean(protectedKind(node,attributesOf(node))||node.shadowRoots?.length||node.contentDocument||(node.childNodeCount??0)>(node.children?.length??0)||node.children?.some(mayProtect));
const frameEvents=['Page.frameNavigated','Target.attachedToTarget','Target.detachedFromTarget'];
export function regionProtectionChanged(message,sessionId,scope=true){
  const tracker=typeof scope==='object'&&scope!==null;
  if(tracker?!scope.sessions.has(message.sessionId):message.sessionId!==sessionId)return false;
  const attribute=['DOM.attributeModified','DOM.attributeRemoved'].includes(message.method);
  // MP-11: an inspected frame that navigates, appears or detaches retires it.
  if(tracker&&frameEvents.includes(message.method))return message.method!=='Target.attachedToTarget'||message.params?.targetInfo?.type==='iframe';
  if(!tracker||!scope.nodes)
    return structural.includes(message.method)||attribute&&(protectionAttributes.has(message.params?.name)||scope&&['style','class'].includes(message.params?.name));
  if(attribute)return protectionAttributes.has(message.params?.name);
  if(message.method==='DOM.childNodeInserted')return mayProtect(message.params?.node);
  if(message.method==='DOM.childNodeRemoved')return scope.nodes.has(message.sessionId+' '+message.params?.nodeId);
  return structural.includes(message.method);
}

export async function captureRegionMasks(connection, sessionId, { reinspect = true, ...options } = {}) {
  // Layout changes or failed metadata checks cannot reveal an unmapped field.
  const inspection = await inspect(connection, sessionId, options);
  let before = await measure(connection, inspection).catch(() => null), beforeAt = performance.timeOrigin + performance.now();
  return { inspection, tracked:inspection.unbounded||inspection.nodes.length>0, get hasRegions(){return before===null||before.length>0;}, get beforeAt(){return beforeAt;}, async afterCapture({ width = geometry.width, height = geometry.height, captured_ms } = {}) {
    if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) throw new Error("Capture geometry unavailable");
    const fullFrame = [{ x: 0, y: 0, width, height }];
    try {
      const after = await measure(connection, reinspect ? await inspect(connection, sessionId, options) : inspection);
      // A readback is bracketed by the previous and this measurement; it
      // may use masks only when both agree and it follows the previous one.
      const bracketed = !Number.isFinite(captured_ms) || captured_ms >= beforeAt;
      if (before===null || after===null || JSON.stringify(before) !== JSON.stringify(after) || !bracketed) {
        // MP-08/MP-11: a re-measured inspection (no new protected node is
        // possible without a retiring event) only advances its bracket;
        // unbounded or re-inspected metadata asks for a fresh inspection.
        if (reinspect || after === null) this.changed = true;
        else { before = after; beforeAt = performance.timeOrigin + performance.now(); }
        return fullFrame;
      }
      // CDP bounds are CSS coordinates; the raster crop masks native PNG pixels.
      return after.map(region => ({ x: region.x * width / geometry.width, y: region.y * height / geometry.height,
        width: region.width * width / geometry.width, height: region.height * height / geometry.height }));
    } catch { this.changed=true;this.failed=true;return fullFrame; }
  } };
}

// MP-08/MP-11: CDP screenshots use the shared per-frame protection transform
// (browser-protection-regions.mjs) on the page's own session: a second
// attached session resets the page's device-metrics emulation (DPR2 would
// silently become DPR1). Native and controller node references are backend
// ids, so its DOM.getDocument cannot stale them. Masks apply only when the
// measurements before and after the screenshot agree and bind its raster;
// otherwise (and for an unknown policy or unbound page) the whole raster is masked.
export async function captureProtectionFence(connection, sessionId, targetId, policy, { record = () => {}, measure = measurePageProtection } = {}) {
  const measured = async () => {
    if (policy.unknown) return null;
    try { return await measure(connection, sessionId, targetId, policy, { hidden: true }); }
    catch (error) { record('fence_unbound ' + (/^MP-11: [a-z -]{1,40}$/.test(error?.message) ? error.message.slice(7) : 'cdp')); return null; }
  };
  const before = await measured();
  return { async afterCapture({ width, height }) {
    const after = await measured();
    // The screenshot raster scales the measured viewport uniformly (view
    // image scale): map regions outward; any other geometry fails closed.
    const sx = width / after?.viewport?.[0], sy = height / after?.viewport?.[1];
    const failed = !before || !after ? 'fence_unbound' : protectionDigest(before) !== protectionDigest(after) ? 'fence_changed' : !(sx > 0) || Math.abs(sx - sy) > 1e-9 ? 'fence_raster' : null;
    if (failed) { record(failed); return [{ x: 0, y: 0, width, height }]; }
    for (const reason of new Set(after.withheld)) record(reason);
    return after.regions.map(([x, y, w, h]) => {
      const left = Math.floor(x * sx), top = Math.floor(y * sy);
      return { x: left, y: top, width: Math.ceil((x + w) * sx) - left, height: Math.ceil((y + h) * sy) - top };
    });
  } };
}

// MP-08/MP-11: a native readback must follow its trusted pre-capture metadata.
// A coalesced sample older than that fence receives an opaque whole-frame mask.
export class NativeRegionProtection {
  constructor(connection,sessionId,options={}){Object.assign(this,{connection,sessionId,options});this.revision=0;this.sessions=new Set([sessionId]);}
  retire(){this.revision++;this.guard=null;}
  // MP-08/MP-11: events that can add a protected node retire this guard;
  // without a guard every structural/protection event of every document
  // inspected so far (including an in-flight inspection) does.
  tracker(){const inspection=this.guard?.inspection;return inspection?{sessions:inspection.sessions,nodes:new Set(inspection.nodes.map(n=>n.sessionId+' '+n.nodeId))}:{sessions:this.sessions,nodes:null};}
  // Readbacks older than the newest trusted measurement are fully masked.
  get beforeAt(){return Math.max(this.fencedAt??-Infinity,this.guard?.beforeAt??-Infinity);}
  async refresh(){
    // Events before a document's own inspection cannot matter; restart the set.
    const revision=this.revision;this.sessions=new Set([this.sessionId]);
    const guard=await captureRegionMasks(this.connection,this.sessionId,{...this.options,reinspect:false,sessions:this.sessions});
    if(revision!==this.revision)throw Error('MP-11: native region fence retired');
    this.guard=guard;this.fencedAt=performance.timeOrigin+performance.now();
  }
  async regions(raw){
    const guard=this.guard;
    if(!guard||!Number.isFinite(raw.captured_ms))throw Error('MP-11: native region fence unavailable');
    // MP-08/MP-10/MP-11: getDocument enables trusted DOM events; LinuxCapture
    // retires this inspection on every event that can add a protected node.
    // Tracked nodes (even hidden ones) are re-measured per readback.
    const result=raw.captured_ms<this.beforeAt?[{x:0,y:0,width:raw.width,height:raw.height}]:guard.tracked?await guard.afterCapture(raw):[];
    if(this.guard!==guard)throw Error('MP-11: native region fence retired');
    // Unbounded metadata keeps masking whole frames and re-inspects at most
    // 4/s; failed metadata re-inspects now and fails closed if it still fails.
    if(guard.changed&&(guard.failed||performance.timeOrigin+performance.now()-this.fencedAt>=250))await this.refresh();else guard.changed=false;
    return result;
  }
}

// MP-08/MP-11: bind masks to a full viewport before any encoder or crop sees
// it. Keep this policy below clients and preserve the ordinary wire shape.
export async function captureProtectedDisplay(host,tab,clip=null,optimizeForSpeed=true){
  const scale=host.scales.get(tab.tab_id)??1;let frame;
  try{
    const {protected_regions,...captured}=await host.screenshot(tab,null,true,'png',optimizeForSpeed);
    frame={...captured,[displayMaskRegions]:protected_regions,data_base64:protected_regions.length?maskPng(captured.data_base64,protected_regions.map(r=>[r.x,r.y,r.width,r.height]),scale):captured.data_base64};
  }catch(error){
    if(['stale_document_reference','browser_action_cancelled'].includes(error?.code))throw error;
    const {connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id);
    await assertCurrentDocument(connection,sessionId,tab.target_id,tab.document_id);
    const width=geometry.width*scale,height=geometry.height*scale;
    frame={generation:host.generation,tab_id:tab.tab_id,document_id:tab.document_id,mime_type:'image/png',width,height,[displayMaskRegions]:[{x:0,y:0,width,height}],data_base64:opaqueFrame(width,height)};
  }
  const cropped=cropProtectedPng(frame.data_base64,clip,scale);
  const full=!clip||clip.width===geometry.width&&clip.height===geometry.height;
  const regions=(frame[displayMaskRegions]??[]).map(r=>({x:(r.x-(full?0:clip.x*scale))*(clip?.scale??1),y:(r.y-(full?0:clip.y*scale))*(clip?.scale??1),width:r.width*(clip?.scale??1),height:r.height*(clip?.scale??1)}));
  return {...frame,...cropped,[displayMaskRegions]:regions,[displayFullMaskRegions]:frame[displayMaskRegions]};
}
