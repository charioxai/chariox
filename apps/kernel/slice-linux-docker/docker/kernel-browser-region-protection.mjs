import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
import {maskPng,opaqueFrame,cropProtectedPng,displayMaskRegions,displayFullMaskRegions} from './kernel-browser-pixels.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';
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
async function inspect(connection, sessionId, mirrorStructured = false) {
  const { root } = await connection.send("DOM.getDocument", { depth: -1, pierce: true }, sessionId);
  const { nodeIds } = await connection.send("DOM.querySelectorAll", {
    nodeId: root.nodeId,
    selector: 'input[type="password"], [data-chariox-secret], [data-chariox-observation-protected], [data-observation-protected], input[autocomplete*="password" i], input[autocomplete*="one-time-code" i], input[autocomplete*="cc-" i], iframe, frame',
  }, sessionId);
  if (!Array.isArray(nodeIds)) throw new Error("Capture protection unavailable");
  // MP-11: structured mirrors may inspect only same-origin nested documents
  // that CDP actually exposes. Other frames keep opaque protection.
  const admittedFrames=new Set();
  if(mirrorStructured) {
    const {frameTree}=await connection.send('Page.getFrameTree',{},sessionId);
    const origin=frameTree?.frame?.securityOrigin;
    const admit=tree=>{if(!origin||origin==='://'||tree.frame.securityOrigin!==origin)return;admittedFrames.add(tree.frame.id);for(const child of tree.childFrames??[])admit(child);};
    if(frameTree)admit(frameTree);
  }
  const kinds=new Map(), pending=[root], exposedFrames=new Set(), explicitlyProtected=new Set();
  let visited=0;
  while (pending.length) {
    if (++visited > 100_000) return {nodes:[],unbounded:true};
    const node = pending.pop();
    for(const shadow of node.shadowRoots??[]){
      if(shadow.shadowRootType==='user-agent')continue;
      if(shadow.shadowRootType==='open')pending.push(shadow);else kinds.set(node.nodeId,'opaque');
    }
    const kind=node.localName?protectedKind(node,attributesOf(node)):null;
    if(kind){kinds.set(node.nodeId,kinds.get(node.nodeId)==='opaque'?'opaque':kind);if(kind!=='frame')explicitlyProtected.add(node.nodeId);}
    if(mirrorStructured&&node.contentDocument&&admittedFrames.has(node.frameId)) {
      exposedFrames.add(node.nodeId);pending.push(node.contentDocument);
    }
    pending.push(...(node.children ?? []));
  }
  // A selector match the walk did not classify stays conservatively unbounded.
  for(const id of nodeIds)if(!kinds.has(id))kinds.set(id,'explicit');
  const nodes=[...kinds].filter(([id])=>!exposedFrames.has(id)||explicitlyProtected.has(id)).map(([nodeId,kind])=>({nodeId,kind})).sort((a,b)=>a.nodeId-b.nodeId);
  if (nodes.length > 1024) return {nodes:[],unbounded:true};
  return {nodes,unbounded:false};
}
const notRendered=/Could not compute box model|Could not find node with given id/;
// Current viewport bounds of an inspection; null when any node is unbounded.
// Frames and form fields without a box (hidden, detached) render nothing.
async function measure(connection, sessionId, inspection) {
  if(inspection.unbounded)return null;
  const rects=await Promise.all(inspection.nodes.map(async({nodeId,kind})=>{
    let quad;
    try{quad=(await connection.send("DOM.getBoxModel", { nodeId }, sessionId)).model?.border;}
    catch(error){if(!notRendered.test(error?.message??''))throw error;return kind==='frame'||kind==='input'?undefined:null;}
    if (!Array.isArray(quad) || quad.length !== 8 || quad.some(n => !Number.isFinite(n))) return null;
    const xs = [quad[0], quad[2], quad[4], quad[6]], ys = [quad[1], quad[3], quad[5], quad[7]];
    // Only on-screen pixels can leak; an empty clip is no region.
    const left=Math.max(0,Math.min(...xs)),top=Math.max(0,Math.min(...ys)),right=Math.min(geometry.width,Math.max(...xs)),bottom=Math.min(geometry.height,Math.max(...ys));
    return right>left&&bottom>top?{ x:left, y:top, width:right-left, height:bottom-top }:undefined;
  }));
  return rects.includes(null)?null:rects.filter(Boolean);
}
async function regions(connection, sessionId, mirrorStructured = false) {
  return await measure(connection,sessionId,await inspect(connection,sessionId,mirrorStructured))??[{x:0,y:0,width:geometry.width,height:geometry.height}];
}

export const protectedHostRegions = (connection, sessionId) => regions(connection, sessionId);

// MP-11: page protection can change without repainting any pixel. Legacy
// callers pass hasRegions; a native tracker narrows the events to those that
// can add a protected node (bounds of tracked nodes are re-measured per frame).
const structural=['DOM.documentUpdated','DOM.childNodeInserted','DOM.childNodeRemoved','DOM.childNodeCountUpdated','DOM.shadowRootPushed','DOM.shadowRootPopped'];
const mayProtect=node=>!node||Boolean(protectedKind(node,attributesOf(node))||node.shadowRoots?.length||node.contentDocument||(node.childNodeCount??0)>(node.children?.length??0)||node.children?.some(mayProtect));
export function regionProtectionChanged(message,sessionId,scope=true){
  if(message.sessionId!==sessionId)return false;
  const attribute=['DOM.attributeModified','DOM.attributeRemoved'].includes(message.method);
  if(typeof scope!=='object'||scope===null)
    return structural.includes(message.method)||attribute&&(protectionAttributes.has(message.params?.name)||scope&&['style','class'].includes(message.params?.name));
  if(attribute)return protectionAttributes.has(message.params?.name);
  if(message.method==='DOM.childNodeInserted')return mayProtect(message.params?.node);
  if(message.method==='DOM.childNodeRemoved')return scope.has(message.params?.nodeId);
  return structural.includes(message.method);
}

export async function captureRegionMasks(connection, sessionId, { mirrorStructured = false, reinspect = true } = {}) {
  // Layout changes or failed metadata checks cannot reveal an unmapped field.
  const inspection = await inspect(connection, sessionId, mirrorStructured);
  let before = await measure(connection, sessionId, inspection).catch(() => null), beforeAt = performance.timeOrigin + performance.now();
  return { inspection, tracked:inspection.unbounded||inspection.nodes.length>0, get hasRegions(){return before===null||before.length>0;}, get beforeAt(){return beforeAt;}, async afterCapture({ width = geometry.width, height = geometry.height, captured_ms } = {}) {
    if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) throw new Error("Capture geometry unavailable");
    const fullFrame = [{ x: 0, y: 0, width, height }];
    try {
      const after = await measure(connection, sessionId, reinspect ? await inspect(connection, sessionId, mirrorStructured) : inspection);
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

// MP-08/MP-11: a native readback must follow its trusted pre-capture metadata.
// A coalesced sample older than that fence receives an opaque whole-frame mask.
export class NativeRegionProtection {
  constructor(connection,sessionId){Object.assign(this,{connection,sessionId});this.revision=0;}
  retire(){this.revision++;this.guard=null;}
  // MP-08/MP-11: events that can add a protected node retire this guard;
  // without a guard every structural/protection event does.
  tracker(){const nodes=this.guard?.inspection?.nodes;return nodes?new Set(nodes.map(n=>n.nodeId)):true;}
  // Readbacks older than the newest trusted measurement are fully masked.
  get beforeAt(){return Math.max(this.fencedAt??-Infinity,this.guard?.beforeAt??-Infinity);}
  async refresh(){
    const revision=this.revision,guard=await captureRegionMasks(this.connection,this.sessionId,{reinspect:false});
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
