import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
import {maskPng,opaqueFrame,cropProtectedPng,displayMaskRegions,displayFullMaskRegions} from './kernel-browser-pixels.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';
// Screenshot-region masks from trusted CDP metadata, never page JavaScript.
async function regions(connection, sessionId, mirrorStructured = false) {
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
  const nodes = [...nodeIds], pending = [root], exposedFrames=new Set(), explicitlyProtected=new Set();
  let visited = 0;
  while (pending.length) {
    if (++visited > 100_000) throw new Error("Capture protection tree limit exceeded");
    const node = pending.pop();
    if (node.shadowRoots?.length) {
      if((!mirrorStructured && node.shadowRoots.some(root=>root.shadowRootType!=='user-agent')) || node.shadowRoots.some(root=>root.shadowRootType==='closed'))nodes.push(node.nodeId);
      if(mirrorStructured)pending.push(...node.shadowRoots.filter(root=>root.shadowRootType==='open'));
    }
    if(mirrorStructured) {
      // Inspect open shadow descendants through trusted CDP metadata, never page
      // scripts. UA shadow roots of ordinary inputs/media are native controls.
      const attrs=new Map();for(let i=0;i<(node.attributes?.length??0);i+=2)attrs.set(node.attributes[i],node.attributes[i+1]);
      if(['data-chariox-secret','data-chariox-observation-protected','data-observation-protected'].some(key=>attrs.has(key)) || node.localName==='input' && (attrs.get('type')?.toLowerCase()==='password'||/password|one-time-code|cc-/i.test(attrs.get('autocomplete')??''))){nodes.push(node.nodeId);explicitlyProtected.add(node.nodeId);}
    }
    if(mirrorStructured&&node.contentDocument&&admittedFrames.has(node.frameId)) {
      exposedFrames.add(node.nodeId);pending.push(node.contentDocument);
    }
    pending.push(...(node.children ?? []));
  }
  if (nodes.length > 1024) throw new Error("Capture protection limit exceeded");
  const result = [];
  for (const nodeId of new Set(nodes.filter(id=>!exposedFrames.has(id)||explicitlyProtected.has(id)))) {
    const { model } = await connection.send("DOM.getBoxModel", { nodeId }, sessionId);
    const quad = model?.border;
    if (!Array.isArray(quad) || quad.length !== 8 || quad.some(n => !Number.isFinite(n))) {
      throw new Error("Capture protection bounds unavailable");
    }
    const xs = [quad[0], quad[2], quad[4], quad[6]], ys = [quad[1], quad[3], quad[5], quad[7]];
    result.push({ x: Math.min(...xs), y: Math.min(...ys), width: Math.max(...xs) - Math.min(...xs), height: Math.max(...ys) - Math.min(...ys) });
  }
  return result;
}

export const protectedHostRegions = (connection, sessionId) => regions(connection, sessionId);

// MP-11: page protection can change without repainting any pixel.
export function regionProtectionChanged(message,sessionId,hasRegions=true){
  return message.sessionId===sessionId&&(
    ['DOM.documentUpdated','DOM.childNodeInserted','DOM.childNodeRemoved','DOM.childNodeCountUpdated','DOM.shadowRootPushed','DOM.shadowRootPopped'].includes(message.method)||
    ['DOM.attributeModified','DOM.attributeRemoved'].includes(message.method)&&(['type','autocomplete','data-chariox-secret','data-chariox-observation-protected','data-observation-protected'].includes(message.params?.name)||hasRegions&&['style','class'].includes(message.params?.name)));
}

// MP-11: declared protection retires old pixels at once. Tree/layout churn
// only wakes a fresh capture whose own before/after check binds its masks.
export function protectionDeclared(message){
  return message.method==='DOM.documentUpdated'||['DOM.attributeModified','DOM.attributeRemoved'].includes(message.method)&&!['style','class'].includes(message.params?.name);
}

export async function captureRegionMasks(connection, sessionId, { mirrorStructured = false } = {}) {
  // Layout changes or failed metadata checks cannot reveal an unmapped field.
  const before = await regions(connection, sessionId, mirrorStructured);
  return { hasRegions:before.length>0, async afterCapture({ width = geometry.width, height = geometry.height } = {}) {
    if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) throw new Error("Capture geometry unavailable");
    const fullFrame = [{ x: 0, y: 0, width, height }];
    try {
      const after = await regions(connection, sessionId, mirrorStructured);
      if (JSON.stringify(before) !== JSON.stringify(after)) {this.changed=true;return fullFrame;}
      // CDP bounds are CSS coordinates; the raster crop masks native PNG pixels.
      return after.map(region => ({ x: region.x * width / geometry.width, y: region.y * height / geometry.height,
        width: region.width * width / geometry.width, height: region.height * height / geometry.height }));
    } catch { this.changed=true;return fullFrame; }
  } };
}

// MP-08/MP-11: a native readback must follow its trusted pre-capture metadata.
// A coalesced sample older than that fence receives an opaque whole-frame mask.
export class NativeRegionProtection {
  constructor(connection,sessionId){Object.assign(this,{connection,sessionId});this.revision=0;}
  retire(){this.revision++;this.guard=null;}
  async refresh(){
    const revision=this.revision,guard=await captureRegionMasks(this.connection,this.sessionId);
    if(revision!==this.revision)throw Error('MP-11: native region fence retired');
    this.guard=guard;this.beforeAt=performance.timeOrigin+performance.now();
  }
  async regions(raw){
    const guard=this.guard;
    if(!guard||!Number.isFinite(raw.captured_ms))throw Error('MP-11: native region fence unavailable');
    // MP-08/MP-10/MP-11: getDocument enables trusted DOM events. LinuxCapture
    // retires this snapshot on every protection/tree change, including during
    // attestation. A stable empty snapshot needs no per-frame DOM transfer.
    const result=raw.captured_ms<this.beforeAt?[{x:0,y:0,width:raw.width,height:raw.height}]:guard.hasRegions?await guard.afterCapture(raw):[];
    if(this.guard!==guard)throw Error('MP-11: native region fence retired');
    if(guard.changed)await this.refresh();return result;
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
