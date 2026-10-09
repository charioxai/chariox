import { BrowserSnapshotError } from './browser-controller-snapshot.mjs';
// MP-08/MP-10/MP-11: Miguel's Vault fill-target model (2026-10-09).
// Only the exact filled plain field contributes pixels. No echo/media/frame scans.
import { createHash } from 'node:crypto';
import { withBrowserFrames, assertBrowserFramesUnchanged } from './browser-controller-frames.mjs';
const FRAME_TIMEOUT_MS = 500;
const retired = new WeakMap();
// Fills between recording and completion: covered while plain, never retired.
const filling = new WeakMap();
const fillKey = target => JSON.stringify([target.target_id,target.document_id,target.node_ref]);
export function pruneBrowserFillTargets(browser,connection) {
  const dead=retired.get(connection);
  for(const [key,target] of browser.fillTargets??[])if(dead?.has(fillKey(target))||target.browser_generation!==browser.browserGeneration)browser.fillTargets.delete(key);
}
const digest = value => createHash('sha256').update(value).digest('hex');
const frameTree = async (connection, sessionId) => (await connection.send('Page.getFrameTree', {}, sessionId)).frameTree;
const visit = (tree, id) => tree.frame.id === id ? tree.frame : (tree.childFrames ?? []).map(child => visit(child,id)).find(Boolean);
const rect = quad => {
  if (!Array.isArray(quad) || quad.length !== 8 || !quad.every(Number.isFinite)) throw Error('MP-11: fill geometry unavailable');
  const xs=quad.filter((_,i)=>i%2===0),ys=quad.filter((_,i)=>i%2===1);
  return [Math.min(...xs),Math.min(...ys),Math.max(...xs)-Math.min(...xs),Math.max(...ys)-Math.min(...ys)];
};
function targetNode(target, entry) {
  const ref=entry.prefix ? (target.node_ref?.startsWith(entry.prefix) ? target.node_ref.slice(entry.prefix.length) : '') : target.node_ref;
  return /^backend:[1-9][0-9]*$/.test(ref??'') ? Number(ref.slice(8)) : null;
}

// Bind before focus/input events; no credential bytes are retained in the target.
export async function recordBrowserFill(connection, options, value, revision) {
  const {sessionId,targetId,documentId,nodeRef,browserGeneration}=options;
  const backendNodeId=Number(nodeRef.slice(8));
  const snapshot=await connection.send('DOMSnapshot.captureSnapshot',{computedStyles:[]},sessionId);
  const document=snapshot.documents?.find(doc=>doc.nodes?.backendNodeId?.includes(backendNodeId));
  const tree=await frameTree(connection,sessionId), frame=document && visit(tree,snapshot.strings[document.frameId]);
  if(!frame)throw new BrowserSnapshotError('stale_element_reference','MP-11: fill document unavailable');
  const frameDocument={frameId:frame.id};
  const previous=await fieldState(connection,{sessionId},frameDocument,backendNodeId);
  const {executionContextId}=await connection.send('Page.createIsolatedWorld',{frameId:frame.id,worldName:'chariox-fill-target'},sessionId);
  const {object}=await connection.send('DOM.resolveNode',{backendNodeId,executionContextId},sessionId);
  if(!object?.objectId)throw new BrowserSnapshotError('stale_element_reference','MP-11: fill target removed');
  try {await connection.send('Runtime.callFunctionOn',{objectId:object.objectId,returnByValue:true,
    functionDeclaration:`function(){ const fields=globalThis.__charioxFilledFields??=new WeakMap();const state={changed:false};fields.set(this,state);
      this.addEventListener('input',event=>{if(event.isTrusted)state.changed=true;}); }`},sessionId);}
  finally {await connection.send('Runtime.releaseObject',{objectId:object.objectId},sessionId).catch(()=>{});}
  const target={kind:'browser',target_id:targetId,document_id:options.trackingDocumentId??documentId,node_ref:options.trackingNodeRef??nodeRef,
    frame_id:frame.id,frame_document_id:frame.loaderId,browser_generation:browserGeneration,
    value_hash:digest(options.action?.append ? previous.value+value : value),fill_revision:revision};
  retired.get(connection)?.delete(fillKey(target));
  if(!filling.has(connection))filling.set(connection,new Set());
  filling.get(connection).add(fillKey(target));return target;
}
export function finishBrowserFill(connection,target) {filling.get(connection)?.delete(fillKey(target));}
async function fieldState(connection,entry,document,backendNodeId) {
  const {executionContextId}=await connection.send('Page.createIsolatedWorld',{frameId:document.frameId,worldName:'chariox-fill-target'},entry.sessionId);
  const {object}=await connection.send('DOM.resolveNode',{backendNodeId,executionContextId},entry.sessionId);
  if(!object?.objectId)return {exists:false};
  try {
    const {result,exceptionDetails}=await connection.send('Runtime.callFunctionOn',{objectId:object.objectId,returnByValue:true,
      functionDeclaration:`function(){ const editable=this.localName==='input'||this.localName==='textarea'||this.isContentEditable;
        return {exists:this.isConnected,editable,contenteditable:this.isContentEditable,changed:globalThis.__charioxFilledFields?.get(this)?.changed===true,password:this.localName==='input'&&this.type==='password',
          value:editable?String(this.isContentEditable?this.textContent:this.value):''}; }`},entry.sessionId);
    if(exceptionDetails||!result?.value)throw Error('MP-11: fill state unavailable');
    return result.value;
  } finally {await connection.send('Runtime.releaseObject',{objectId:object.objectId},entry.sessionId).catch(()=>{});}
}

// Range geometry belongs only to the recorded field, including ordinary text overflow.
// Refuse ambiguous positioned clipping rather than release pixels with unproved coverage.
function contenteditableTextGeometry() {
  const range=this.ownerDocument.createRange();range.selectNodeContents(this);
  const viewport=[this.ownerDocument.defaultView.innerWidth,this.ownerDocument.defaultView.innerHeight];
  let clips=[0,0,...viewport],positioned=false;
  for(let element=this;element;element=element.parentElement??element.getRootNode().host) {
    const style=this.ownerDocument.defaultView.getComputedStyle(element);
    const clipX=style.overflowX!=='visible',clipY=style.overflowY!=='visible';
    // Visibility inherits, but descendants may restore visible text. Range
    // bounds conservatively cover that text; only display:none hides a subtree.
    if(style.display==='none')return {rects:[],viewport};
    if(clipX||clipY) {
      if(positioned)throw Error('uncertain positioned clip');
      const box=element.getBoundingClientRect();
      const margin=parseFloat(style.overflowClipMargin)||0;
      // Border bounds conservatively enclose the painted overflow clip.
      const scale=Math.max(1,box.width/(element.offsetWidth||box.width),box.height/(element.offsetHeight||box.height));
      if(clipX){clips[0]=Math.max(clips[0],box.left-margin*scale);clips[2]=Math.min(clips[2],box.right+margin*scale);}
      if(clipY){clips[1]=Math.max(clips[1],box.top-margin*scale);clips[3]=Math.min(clips[3],box.bottom+margin*scale);}
    }
    positioned ||= style.position==='absolute'||style.position==='fixed';
  }
  const rects=[];
  for(const box of range.getClientRects()) {
    const left=Math.max(clips[0],box.left),top=Math.max(clips[1],box.top),right=Math.min(clips[2],box.right),bottom=Math.min(clips[3],box.bottom);
    if(right>left&&bottom>top)rects.push([left,top,right-left,bottom-top]);
  }
  return {rects,viewport};
}
async function fieldTextGeometry(connection,entry,frameId,backendNodeId) {
  const {executionContextId}=await connection.send('Page.createIsolatedWorld',{frameId,worldName:'chariox-fill-target'},entry.sessionId);
  const {object}=await connection.send('DOM.resolveNode',{backendNodeId,executionContextId},entry.sessionId);
  if(!object?.objectId)throw Error('MP-11: fill text geometry unavailable');
  try {
    const {result,exceptionDetails}=await connection.send('Runtime.callFunctionOn',{objectId:object.objectId,returnByValue:true,functionDeclaration:contenteditableTextGeometry.toString()},entry.sessionId);
    if(exceptionDetails||!result?.value)throw Error('MP-11: fill text geometry unavailable');
    return result.value;
  }finally{await connection.send('Runtime.releaseObject',{objectId:object.objectId},entry.sessionId).catch(()=>{});}
}

export async function measurePageProtection(connection,sessionId,targetId,policy,{includeHidden=false,onPlainField}={}) {
  const top=await frameTree(connection,sessionId);
  const {executionContextId}=await connection.send('Page.createIsolatedWorld',{frameId:top.frame.id,worldName:'chariox-fill-viewport'},sessionId);
  const {result}=await connection.send('Runtime.evaluate',{contextId:executionContextId,returnByValue:true,
    expression:'({dpr:devicePixelRatio,width:innerWidth,height:innerHeight,visible:document.visibilityState==="visible"})'},sessionId);
  const metrics=result?.value;
  if(!metrics||!(metrics.dpr>0)||!(metrics.width>0)||!(metrics.height>0))throw Error('MP-11: capture viewport unavailable');
  if(!includeHidden&&!metrics.visible)return null;
  const {cssVisualViewport:visual}=await connection.send('Page.getLayoutMetrics',{},sessionId);
  const page={url:top.frame.url,document_id:top.frame.loaderId,dpr:metrics.dpr,zoom:visual?.zoom??1,viewport:[Math.round(metrics.width*metrics.dpr),Math.round(metrics.height*metrics.dpr)],regions:[],withheld:[]};
  const dead=retired.get(connection)??new Set();retired.set(connection,dead);
  const targets=(policy.targets??[]).filter(t=>t.target_id===targetId&&typeof t.value_hash==='string'&&!t.echo_only&&!dead.has(fillKey(t)));
  if(!targets.length)return page;
  await withBrowserFrames(connection,sessionId,targetId,top.frame.loaderId,async frames=>{
    const seen=new Set(),transforms=new Map([[frames[0],point=>point]]);
    const toViewport=async entry=>{
      if(transforms.has(entry))return transforms.get(entry);
      const up=await toViewport(entry.parent);
      const owner=await connection.send('DOM.getFrameOwner',{frameId:entry.frame.id},entry.parent.sessionId);
      const {model}=await connection.send('DOM.getBoxModel',{backendNodeId:owner.backendNodeId},entry.parent.sessionId);
      const quad=model.content;rect(quad);
      const {executionContextId}=await connection.send('Page.createIsolatedWorld',{frameId:entry.frame.id,worldName:'chariox-fill-viewport'},entry.sessionId);
      const {result}=await connection.send('Runtime.evaluate',{contextId:executionContextId,returnByValue:true,expression:'[innerWidth,innerHeight]'},entry.sessionId);
      const [width,height]=result.value??[];
      if(!(width>0&&height>0)||Math.abs(quad[0]+quad[4]-quad[2]-quad[6])>.1||Math.abs(quad[1]+quad[5]-quad[3]-quad[7])>.1)throw Error('MP-11: fill frame transform unavailable');
      const map=([x,y])=>up([quad[0]+x*(quad[2]-quad[0])/width+y*(quad[6]-quad[0])/height,
        quad[1]+x*(quad[3]-quad[1])/width+y*(quad[7]-quad[1])/height]);
      transforms.set(entry,map);return map;
    };
    for(const entry of frames) {
      const candidates=targets.filter(t=>t.document_id===top.frame.loaderId&&targetNode(t,entry));
      if(!candidates.length)continue;
      const snapshot=await connection.send('DOMSnapshot.captureSnapshot',{computedStyles:[]},entry.sessionId);
      for(const target of candidates) {
        const key=fillKey(target),backendNodeId=targetNode(target,entry);
        const raw=snapshot.documents?.find(doc=>doc.nodes?.backendNodeId?.includes(backendNodeId));
        if(!raw)continue;
        const frameId=snapshot.strings[raw.frameId],frame=visit(entry.tree,frameId);
        if(!frame || target.frame_id&&target.frame_id!==frameId || target.frame_document_id&&target.frame_document_id!==frame.loaderId)continue;
        seen.add(key);
        const state=await fieldState(connection,entry,{frameId},backendNodeId);
        const inFlight=filling.get(connection)?.has(key)&&state.exists&&state.editable;
        if(!inFlight&&(!state.exists||state.changed||!state.editable||!state.value || digest(state.value)!==target.value_hash)) {dead.add(key);continue;}
        if(state.password)continue; // Rechecked even for a previously plain field.
        // MP-08/MP-11: DOMSnapshot includes retained non-rendered fields. Only
        // confirmed absence of layout for the entire field subtree permits
        // skipping box lookup; keep identity tracking so reveal remasks it.
        const nodeIndex=raw.nodes.backendNodeId.indexOf(backendNodeId),parents=raw.nodes.parentIndex;
        if(!Array.isArray(raw.layout?.nodeIndex)||!Array.isArray(parents))throw Error('MP-11: fill layout unavailable');
        const rendered=raw.layout.nodeIndex.some(index=>{
          for(let steps=0;index>=0&&steps<parents.length;steps++,index=parents[index])if(index===nodeIndex)return true;
          return false;
        });
        if(!rendered)continue;
        await onPlainField?.({sessionId:entry.sessionId,backendNodeId});
        const {model}=await connection.send('DOM.getBoxModel',{backendNodeId},entry.sessionId);
        const map=await toViewport(entry),fieldRegions=[];
        const addQuad=(localQuad,transform)=>{
          const quad=[];for(let i=0;i<8;i+=2)quad.push(...transform(localQuad.slice(i,i+2)));
          const [x,y,w,h]=rect(quad),s=metrics.dpr;
          if(w<=0||h<=0)return;
          const left=Math.max(0,Math.floor(x*s)),upper=Math.max(0,Math.floor(y*s));
          const right=Math.min(page.viewport[0],Math.ceil((x+w)*s)),bottom=Math.min(page.viewport[1],Math.ceil((y+h)*s));
          if(right>left&&bottom>upper)fieldRegions.push([left,upper,right-left,bottom-upper]);
        };
        addQuad(model.border,map);
        if(state.contenteditable) {
          const geometry=await fieldTextGeometry(connection,entry,frameId,backendNodeId);
          let textMap=map;
          // Same-renderer child documents report Range bounds in their own CSS viewport.
          // CDP frame-owner quads already place that viewport in this renderer's coordinates.
          if(frameId!==entry.frame.id) {
            const owner=await connection.send('DOM.getFrameOwner',{frameId},entry.sessionId);
            const {model:frameModel}=await connection.send('DOM.getBoxModel',{backendNodeId:owner.backendNodeId},entry.sessionId);
            const quad=frameModel.content,[width,height]=geometry.viewport;rect(quad);
            if(!(width>0&&height>0)||Math.abs(quad[0]+quad[4]-quad[2]-quad[6])>.1||Math.abs(quad[1]+quad[5]-quad[3]-quad[7])>.1)throw Error('MP-11: fill text transform unavailable');
            textMap=([x,y])=>map([quad[0]+x*(quad[2]-quad[0])/width+y*(quad[6]-quad[0])/height,quad[1]+x*(quad[3]-quad[1])/width+y*(quad[7]-quad[1])/height]);
          }
          for(const [x,y,w,h] of geometry.rects)addQuad([x,y,x+w,y,x+w,y+h,x,y+h],textMap);
        }
        if(fieldRegions.length) {
          const left=Math.min(...fieldRegions.map(r=>r[0])),upper=Math.min(...fieldRegions.map(r=>r[1]));
          const right=Math.max(...fieldRegions.map(r=>r[0]+r[2])),bottom=Math.max(...fieldRegions.map(r=>r[1]+r[3]));
          page.regions.push([left,upper,right-left,bottom-upper]);
        }
      }
    }
    for(const target of targets)if(!seen.has(fillKey(target)))dead.add(fillKey(target));
    await assertBrowserFramesUnchanged(connection,frames);
  });
  return page;
}

export async function measureBrowserProtection(browser,policy) {
  if(policy?.unknown||!Array.isArray(policy?.targets))throw Error('MP-11: fill policy unavailable');
  // MP-08/MP-10/MP-11: registration alone contributes no browser pixels.
  // Pending fills enter the private map before input and keep the full fence.
  const filled=policy.targets.some(target=>target.kind==='browser'&&typeof target.value_hash==='string');
  if(!filled&&!(browser?.fillTargets?.size))return {pages:[]};
  const connection=await browser.ensureConnection(),{targetInfos=[]}=await connection.send('Target.getTargets');
  const pages=[];
  for(const info of targetInfos.filter(t=>t.type==='page')) {
    const {bounds,windowId}=await connection.send('Browser.getWindowForTarget',{targetId:info.targetId});
    if(bounds?.windowState==='minimized')continue;
    const {sessionId}=await connection.send('Target.attachToTarget',{targetId:info.targetId,flatten:true});
    try {
      const supplied=policy.targets.filter(t=>t.target_id===info.targetId);
      const tracked=[...(browser.fillTargets?.values()??[])].filter(t=>t.target_id===info.targetId);
      const targets=supplied.map(t=>tracked.find(own=>own.node_ref===t.node_ref&&own.document_id===t.document_id)??t);
      for(const target of tracked)if(!targets.includes(target))targets.push(target);
      const measured=await measurePageProtection(connection,sessionId,info.targetId,{...policy,targets});
      if(measured)pages.push({target_id:info.targetId,window_id:windowId,window:[bounds.left,bounds.top,bounds.width,bounds.height],chrome:false,...measured});
    } finally {await connection.send('Target.detachFromTarget',{sessionId}).catch(()=>{});}
  }
  pruneBrowserFillTargets(browser,connection);return {pages};
}

async function withSession(connection, targetId, run) {
  const { sessionId } = await connection.send('Target.attachToTarget', { targetId, flatten: true });
  try { return await run(sessionId); }
  finally { await connection.send('Target.detachFromTarget', { sessionId }).catch(() => {}); }
}

export const protectionDigest = measurement => createHash('sha256').update(JSON.stringify(measurement)).digest('hex');

// The compositor may still show the layout preceding a measurement: after
// two animation frames, pixels derive from that layout or later. One frame
// suffices before re-measuring: it applies pending compositor scroll offsets.
export async function awaitPresented(browser, pages, frames = 2) {
  if(!pages.length)return;
  const nested = 'requestAnimationFrame(() => resolve(true))';
  const callback = frames === 2 ? `requestAnimationFrame(() => ${nested})` : nested;
  const connection = await browser.ensureConnection();
  await Promise.all(pages.map(page => withSession(connection, page.target_id, async sessionId => {
    const top = await frameTree(connection, sessionId);
    const { executionContextId } = await connection.send('Page.createIsolatedWorld', { frameId: top.frame.id, worldName: 'chariox-protection-regions' }, sessionId);
    const { result } = await connection.send('Runtime.evaluate', { contextId: executionContextId, awaitPromise: true, returnByValue: true,
      expression: `new Promise(resolve => { setTimeout(() => resolve(false), ${FRAME_TIMEOUT_MS}); ${callback}; })` }, sessionId);
    if (result?.value !== true) throw new Error('MP-11: page did not present a frame');
  })));
}

// A measurement whose layout has reached the screen, or null (fail closed).
// Measuring again after a presented frame also observes compositor scrolls.
export async function measurePresented(browser, policy) {
  try {
    const measurement = await measureBrowserProtection(browser, policy);
    await awaitPresented(browser, measurement.pages);
    return measurement;
  } catch { return null; }
}

// Measure, let the measured layout reach the screen, capture, re-measure. A
// changed page, window, frame tree or region set retries with a fresh
// measurement; after `attempts` it refuses the capture rather than inventing a visual mask.
export async function fenceBrowserCapture(browser, policy, capture, attempts = 3) {
  for (let attempt = 0; attempt < attempts; attempt++) {
    const before = await measurePresented(browser, policy);
    if (!before) break;
    const result = await capture(before);
    // One frame first: the re-measurement then sees compositor scrolls too.
    let after = null;
    try { await awaitPresented(browser, before.pages, 1); after = await measureBrowserProtection(browser, policy); } catch {}
    if (after && protectionDigest(after) === protectionDigest(before)) return result;
  }
  throw Error('MP-11: fill-target capture fence unavailable');
}

// Streams: a frame captured with protection serial S is released only when a
// measurement that began after its capture still equals S. New protection is
// adopted only after two equal measurements separated by a presented frame
// (measure() awaits one first); meanwhile serial 0 refuses release.
export class ProtectionGate {
  constructor(measure) { this.measure = measure; this.serial = 0; this.current = null; this.candidate = null; }
  get protection() { return this.current?.measurement ?? null; }
  get protectionSerial() { return this.current?.serial ?? 0; }
  async step(now = Date.now) {
    const started = now();
    const measurement = await this.measure().catch(() => null);
    const digest = measurement ? protectionDigest(measurement) : null;
    const verified = digest && digest === this.current?.digest ? this.current.serial : null;
    let changed = false;
    if (digest !== (this.current?.digest ?? null)) {
      const stable = digest !== null && digest === this.candidate;
      changed = stable || this.current !== null;
      this.current = stable ? { serial: ++this.serial, digest, measurement } : null;
      this.candidate = digest;
    }
    return { started, verified, changed };
  }
}
