import {inspectMirrorCustomElements} from './kernel-browser-mirror-custom-elements.mjs';
// MP-08/MP-10/MP-11: bounded, caller/document/policy-bound mirroring service.
import {mirrorChunks,mirrorFrameBytes} from './kernel-browser-mirror-wire.mjs';
import { losslessRegion } from './kernel-browser-display.mjs';
import { timestamp } from './kernel-browser-timing.mjs';
import { randomUUID } from 'node:crypto';
import { mirrorInitialStyles } from './kernel-browser-mirror-styles.mjs';
import { mirrorObserverExpression } from './kernel-browser-mirror-observer.mjs';
import { materializeMirrorResources,MirrorTreeHasher } from './kernel-browser-mirror-resources.mjs';
import {materializeMirrorLocalFonts} from './kernel-browser-mirror-local-fonts.mjs';
import { observationProtectedVariants } from './browser-controller-snapshot.mjs';
import { locateBrowserRegions } from './browser-observation-regions.mjs';
import { assertCurrentDocument,assertNotCancelled } from './browser-controller-actions.mjs';
import { captureRegionMasks } from './kernel-browser-region-protection.mjs';
import { decodePng,maskPixels } from './kernel-browser-pixels.mjs';

const lifetime=60000,maxWire=mirrorFrameBytes;
const videoSnapshot=()=>({root:'n9007199254740991',nodes:[{id:'n9007199254740991',parent:null,children:['n9007199254740988','n9007199254740990'],kind:'element',tag:'html',style:{margin:'0px'}},{id:'n9007199254740988',parent:'n9007199254740991',children:[],kind:'element',tag:'head'},{id:'n9007199254740990',parent:'n9007199254740991',children:['n9007199254740989'],kind:'element',tag:'body',style:{margin:'0px'}},{id:'n9007199254740989',parent:'n9007199254740990',children:[],kind:'tile',tag:'div',box:{x:0,y:0,width:1280,height:800},reason:'observer_bounds_or_unavailable'}],resources:[],fonts:[],scroll:{x:0,y:0},focused:null,selection:null});
// Trusted admission error: never constructed from page/CDP error strings.
export class MirrorInputEpochRefusal extends Error {
  constructor() { super('MP-11: stale mirror input epoch'); }
}
export class MirrorFrameChanged extends Error {
  constructor() { super('MP-11: mirror frame changed before commit'); }
}
export class MirrorService {
  constructor(host) {this.host=host;this.now=()=>performance.now();this.streams=new Map();this.expiry=setInterval(()=>this.expire(),5000);this.expiry.unref?.();}
  invalidate() {for(const stream of this.streams.values()){void stream.frameCustom?.release();stream.frameCustom=null;stream.pending=null;stream.previous=null;stream.observed=null;stream.resources.clear();stream.cache.clear();stream.policy=null;stream.geometryResetPolicy=null;stream.epochs=[];stream.refinePending=false;}}
  clear() {for(const stream of this.streams.values())void stream.frameCustom?.release();this.streams.clear();}
  removeTab(tabId) {for(const [id,s] of this.streams)if(s.tab_id===tabId)this.drop(id);}
  drop(id) {const stream=this.streams.get(id);void stream?.frameCustom?.release();this.streams.delete(id);}
  expire() {for(const [id,s] of this.streams)if(Date.now()>s.expires){void s.frameCustom?.release();this.streams.delete(id);}}
  require(id,scope,generation) {
    this.expire();const stream=this.streams.get(id);
    if(!stream || stream.scope!==scope || generation!==this.host.generation) throw new Error('MP-11: stale or foreign mirror');
    stream.expires=Date.now()+lifetime;return stream;
  }
  assertWebTab(tab) {
    if([...(this.host.browser.appTabs?.apps?.values()??[])].some(app=>app.targetId===tab.target_id))throw new Error('MP-11: App views require the native App capability path');
  }
  async world(tab) {
    const {connection,sessionId}=await this.host.browser.resolvePageTarget(tab.target_id);
    const {frameTree}=await connection.send('Page.getFrameTree',{},sessionId);
    if(frameTree?.frame?.loaderId!==tab.document_id)throw new Error('MP-11: stale mirror document');
    const world=await this.host.browser.ensureFocusWorld(connection,sessionId,tab.target_id,frameTree.frame);
    if(!Number.isSafeInteger(world.contextId)||world.contextId<=0) throw new Error('MP-11: mirror isolated world unavailable');
    if(!world.mirrorInstalled) {
      this.initialStyles??=mirrorInitialStyles(this.host.browser,connection);
      const initial=await this.initialStyles;
      const installed=await connection.send('Runtime.evaluate',{expression:mirrorObserverExpression(initial),contextId:world.contextId,returnByValue:true},sessionId);
      if(installed.exceptionDetails||installed.result?.value!==true) throw new Error('MP-11: mirror observer unavailable');
      world.mirrorInstalled=true;
    }
    return {connection,sessionId,contextId:world.contextId};
  }
  async evaluate(world,expression) {
    const reply=await world.connection.send('Runtime.evaluate',{expression,contextId:world.contextId,returnByValue:true},world.sessionId);
    if(reply.exceptionDetails||!Object.hasOwn(reply.result??{},'value')) throw new Error('MP-11: mirror observation unavailable');
    return reply.result.value;
  }
  async subscribe(command,scope) {
    this.expire();if(this.streams.size>=8||![1,2].includes(command.device_scale_factor))throw new Error('MP-11: mirror negotiation bounds');
    const tab=await this.host.target(command),scale=this.host.scales.get(tab.tab_id);
    if(scale && scale!==command.device_scale_factor)throw new Error('MP-08: canonical mirror geometry already selected');
    this.assertWebTab(tab);const {connection,sessionId}=await this.host.browser.resolvePageTarget(tab.target_id);
    await connection.send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:command.device_scale_factor,mobile:false},sessionId);
    this.host.scales.set(tab.tab_id,command.device_scale_factor);
    const subscription_id=`host-mirror-${randomUUID()}`;
    this.streams.set(subscription_id,{scope,tab_id:tab.tab_id,sequence:0,epochs:[],inputCustomFingerprint:'[]',previous:null,resources:new Map(),cache:new Map(),hasher:new MirrorTreeHasher(),fallback:new Set(),expires:Date.now()+lifetime,policy:null});
    return {subscription_id,generation:this.host.generation,tab_id:tab.tab_id,device_scale_factor:command.device_scale_factor};
  }
  async next(command,scope,options={}) {
    const stream=this.require(command.subscription_id,scope,command.generation);
    if(stream.busy)throw new Error('MP-11: mirror credit already outstanding');
    stream.busy=true;
    try {
      const cursor=command.after_chunk??null;
      if(cursor!==null&&(!Number.isSafeInteger(cursor)||cursor<0))throw Error('MP-11: invalid mirror chunk cursor');
      if(stream.pending) {
        const tab=await this.host.displayTarget({tab_id:stream.tab_id,generation:command.generation});
        if(stream.pending.policy!==this.host.protection||stream.pending.document_id!==tab.document_id) {
          await stream.frameCustom?.release();stream.frameCustom=null;stream.pending=null;stream.previous=null;stream.observed=null;stream.epochs=[];
          throw Error('MP-11: mirror chunk policy or document changed');
        }
        const pending=stream.pending;await stream.frameCustom?.verify();
        if(stream.pending!==pending||pending.policy!==this.host.protection||this.host.generation!==command.generation||this.streams.get(command.subscription_id)!==stream)throw Error('MP-11: mirror chunk policy or document changed');
        if(pending.sourceRevision!==null&&await this.evaluate(pending.world,'globalThis.__charioxMirror.epoch()')!==pending.sourceRevision)throw new MirrorFrameChanged();
        if(command.after_sequence===pending.sequence&&cursor===null){await stream.frameCustom?.release();stream.frameCustom=null;stream.pending=null;}
        else {
          if(command.after_sequence!==pending.base_sequence)throw Error('MP-11: invalid mirror chunk base');
          const index=cursor===null?0:cursor+1;
          if(index>=pending.chunks.length)throw Error('MP-11: invalid mirror chunk acknowledgement');
          if(index===pending.chunks.length-1){pending.lastIssued=true;const epoch=stream.epochs.find(e=>e.sequence===pending.sequence);if(epoch)epoch.issuedAt=this.now();}
          return pending.chunks[index];
        }
      }
      if(cursor!==null)throw Error('MP-11: mirror chunk credit has no frame');
      let packet,chunks;
      try{packet=await this.readPacket(command,scope,options);chunks=mirrorChunks(packet)}catch(error){
        // Packing failure must not leave a guessable, unissued input epoch/base.
        await stream.frameCustom?.release();stream.frameCustom=null;stream.pending=null;stream.previous=null;stream.observed=null;stream.epochs=[];stream.resources.clear();stream.cache.clear();stream.policy=null;
        throw error;
      }
      if(!chunks){await stream.frameCustom?.release();stream.frameCustom=null;return packet;}
      stream.pending={chunks,sequence:packet.sequence,base_sequence:command.after_sequence,document_id:packet.document_id,policy:this.host.protection,world:stream.frameWorld,sourceRevision:stream.frameSourceRevision,lastIssued:chunks.length===1};
      return chunks[0];
    }catch(error){
      // Any delayed native/protection fence failure revokes the retained frame,
      // including its input epochs. A later credit starts from a fresh base.
      await stream.frameCustom?.release();stream.frameCustom=null;stream.pending=null;stream.previous=null;stream.observed=null;stream.epochs=[];stream.resources.clear();stream.cache.clear();stream.policy=null;
      stream.geometryResetPolicy=error instanceof MirrorFrameChanged?this.host.protection:null;
      throw error;
    }finally{stream.busy=false;}
  }
  async readPacket(command,scope,{signal}={}) {
    const started=timestamp();let stage=started;
    const mark=name=>{this.host.timing?.(`mirror_${name}`,stage);stage=timestamp();};
    const stream=this.require(command.subscription_id,scope,command.generation),tab=await this.host.displayTarget({tab_id:stream.tab_id,generation:command.generation});
    mark('target');
    this.assertWebTab(tab);assertNotCancelled(signal);const world=await this.world(tab),policy=this.host.protection;
    mark('world');
    if(stream.document_id!==tab.document_id) {stream.previous=null;stream.observed=null;stream.epochs=[];stream.refinePending=false;stream.resources.clear();stream.cache.clear();stream.fallback.clear();}
    if(stream.policy!==policy) {stream.previous=null;stream.observed=null;stream.epochs=[];stream.refinePending=false;stream.resources.clear();stream.cache.clear();}
    if(!Array.isArray(command.drift_nodes)||command.drift_nodes.length>64||command.drift_nodes.some(id=>!stream.previous?.nodes.some(n=>n.id===id&&n.kind==='element'))) throw new Error('MP-11: invalid drift report');
    for(const id of command.drift_nodes)stream.fallback.add(id);
    // Registered Vault target geometry and plaintext/media echoes use the SAME
    // trusted CDP locator as screenshots. A failed locator fences DOM output.
    const targets=policy.targets.filter(t=>t.kind==='browser'&&t.target_id===tab.target_id);
    const regions=targets.length?await locateBrowserRegions(targets,this.host.browser,policy.values,{contentTarget:tab.target_id,contentScale:this.host.scales.get(tab.tab_id)??1}):[];
    mark('regions');
    await stream.frameCustom?.release();stream.frameCustom=await inspectMirrorCustomElements(world);
    let source;stream.fullFallback=false;
    try {source=await this.evaluate(world,`globalThis.__charioxMirror.read(${JSON.stringify(observationProtectedVariants(policy.values))},${JSON.stringify(regions)},${JSON.stringify(command.subscription_id)},${!stream.observed})`);}catch {
      await assertCurrentDocument(world.connection,world.sessionId,tab.target_id,tab.document_id);
      // Bounded/unsupported DOM becomes the existing protected full video region.
      // Synthetic tile IDs never authorize element input into the original page.
      stream.fullFallback=true;
      source=videoSnapshot();
    }
    if(source.styles) {
      if(!Array.isArray(source.styles)||source.styles.length>100000)throw new Error('MP-11: invalid private CSS palette');
      source.nodes=source.nodes.map(node=>{if(node.style_index===undefined)return node;const {style_index,...record}=node;if(!Number.isSafeInteger(style_index)||!source.styles[style_index])throw new Error('MP-11: invalid private CSS reference');return {...record,style:source.styles[style_index]};});
      delete source.styles;
    }
    if(source.incremental) {
      if(!stream.observed)throw new Error('MP-11: mirror observer lost base');
      const records=new Map(stream.observed.nodes.map(n=>[n.id,n]));
      for(const id of source.removed)records.delete(id);
      for(const n of source.nodes)records.set(n.id,n);
      const ordered=[],visit=id=>{const n=records.get(id);if(!n||ordered.length>=100000)throw new Error('MP-11: invalid observer delta');ordered.push(n);for(const child of n.children)visit(child);};visit(source.root);
      source.nodes=ordered;
    }
    delete source.incremental;delete source.removed;
    // Keep the isolated observer's sanitized resource descriptors unmodified.
    // Public resource remapping, drift tiles and hidden-subtree removal occur below.
    stream.observed=stream.fullFallback?null:source;
    const styleCopies=new WeakMap();
    const cloneStyle=style=>{if(Object.values(style).some(value=>value.startsWith('resource:')))return {...style};if(!styleCopies.has(style))styleCopies.set(style,{...style});return styleCopies.get(style);};
    source={...source,nodes:source.nodes.map(n=>({...n,children:[...n.children],...(n.style?{style:cloneStyle(n.style)}:{})}))};
    mark('snapshot');
    // Observer supplies only sanitized content; original resource URLs remain private.
    const material=await materializeMirrorResources(world.connection,world.sessionId,source.resources,policy.values,stream.cache,{loadedFontBody:url=>this.host.browser.mirrorFonts?.read(world.connection,world.sessionId,url,tab.document_id)});
    mark('resources');
    for(const node of source.nodes) {
      if(node.resource) {node.resource=material.mapped.get(node.resource)??undefined;if(!node.resource){node.kind='tile';node.tag='img';node.reason='resource_unavailable';}}
      for(const [key,value] of Object.entries(node.style??{})) if(value.startsWith('resource:')) {
        const rid=material.mapped.get(value.slice(9));if(rid)node.style[key]=`resource:${rid}`;else {node.style[key]='none';node.kind='tile';node.tag='img';node.reason='resource_unavailable';}
      }
      if(stream.fallback.has(node.id)&&node.kind==='element'){node.kind='tile';node.tag='img';node.reason='layout_drift';}
    }
    const family=font=>font.family.replaceAll('"','').replaceAll("'",'').trim().toLowerCase();
    // A stylesheet declares many faces (weights/unicode ranges) that Chromium
    // never loads. One unrequested face must not rasterize the entire inherited
    // family when a loaded face is admitted. Real missing glyph/weight geometry
    // still takes the ordinary per-node drift/protected-region path.
    const admittedFamilies=new Set(source.fonts.filter(f=>material.mapped.get(f.resource)).map(family));
    const unavailableFonts=[...new Set(source.fonts.map(family))].filter(f=>!admittedFamilies.has(f));
    for(const node of source.nodes)if(node.kind==='element'&&unavailableFonts.some(f=>(node.style?.['font-family']??'').split(',').some(value=>value.replaceAll('"','').replaceAll("'",'').trim().toLowerCase()===f))){node.kind='tile';node.tag='img';node.reason='font_unavailable';}
    source.fonts=source.fonts.flatMap(f=>{const resource=material.mapped.get(f.resource);return resource?[{...f,resource}]:[];});
    const localFonts=stream.fullFallback?{resources:[],fonts:[]}:await materializeMirrorLocalFonts(world,source,policy.values,{wireBudget:16*1024*1024-material.encodedBytes,decodedBudget:64*1024*1024-material.decodedBytes});
    for(const resource of localFonts.resources)material.resources.set(resource.resource_id,resource);
    source.fonts.push(...localFonts.fonts);
    const sourceRevision=source.revision??0,sourceEpoch=source.revision??null,animating=source.animating??false;
    delete source.animating;
    delete source.resources;delete source.revision;source.selection??=null;
    // Compositor overlays contain already-composited source pixels; retain the
    // inert native control's original layout and CSS transform underneath.
    const compositingNodes=new Map(source.nodes.map(n=>[n.id,n]));
    // Invisible native controls (including Wikipedia's hidden menu controls)
    // need no pixels and cannot force an otherwise mirrorable page to fallback.
    // Recompute every packet: becoming partially visible still fails closed.
    const invisible=node=>{for(let e=node;e;e=compositingNodes.get(e.parent))if(e.style?.opacity&&Number(e.style.opacity)===0)return true;return false;};
    const invisibleTiles=new Set(source.nodes.filter(n=>n.kind==='tile'&&invisible(n)).map(n=>n.id));

    if(['tile','mask'].includes(source.nodes.find(n=>n.id===source.root)?.kind)){stream.fullFallback=true;source=videoSnapshot();delete source.resources;}
    // A tile is an opaque subtree. Descendants must not remain in the wire/map.
    const byId=new Map(source.nodes.map(n=>[n.id,n])),hidden=new Set();
    const hide=id=>{hidden.add(id);for(const child of byId.get(id)?.children??[])hide(child);};
    for(const n of source.nodes)if(n.kind==='mask'||n.kind==='tile'&&n.reason!=='unsupported_paint'){for(const child of n.children)hide(child);n.children=[];}
    source.nodes=source.nodes.filter(n=>!hidden.has(n.id));
    if(source.selection&&!source.nodes.some(n=>n.id===source.selection.anchor_id&&n.kind==='text')||source.selection&&!source.nodes.some(n=>n.id===source.selection.focus_id&&n.kind==='text'))source.selection=null;
    if(source.focused&&!source.nodes.some(n=>n.id===source.focused&&n.kind!=='mask'))source.focused=null;
    // MP-10/MP-11: permanently opaque foreign/closed regions render protected
    // placeholders; they need no source pixels or compositor crop bandwidth.
    const globalBox=node=>{const box={...node.box};for(let ancestor=byId.get(node.parent);ancestor;ancestor=byId.get(ancestor.parent))if(ancestor.kind==='frame') {box.x+=ancestor.box.x+(parseFloat(ancestor.style?.['border-left-width'])||0)+(parseFloat(ancestor.style?.['padding-left'])||0);box.y+=ancestor.box.y+(parseFloat(ancestor.style?.['border-top-width'])||0)+(parseFloat(ancestor.style?.['padding-top'])||0);}return box;};
    const tiles=source.nodes.filter(n=>{if(n.kind!=='tile'||invisibleTiles.has(n.id)||['cross_origin_frame','opaque_shadow'].includes(n.reason)||!(n.box?.width>0&&n.box?.height>0))return false;const b=globalBox(n);return b.x<1280&&b.y<800&&b.x+b.width>0&&b.y+b.height>0;});
    const tileBoxes=new Map(tiles.map(n=>[n.id,globalBox(n)]));
    if(tiles.length>64)throw new Error('MP-11: visible tile limit; use display fallback');
    mark('sanitize');
    let tileFrame=null;
    const nativeOnly=tiles.every(n=>n.reason==='native_control'),nativeRasterKey=JSON.stringify([stream.hasher.hash(source),sourceRevision,this.host.inputEpochs?.get(tab.tab_id)??0,tab.document_id]);
    const reuseNative=nativeOnly&&!animating&&stream.nativeSettled&&stream.nativeRasterKey===nativeRasterKey&&stream.nativeRasterPolicy===policy;
    if(reuseNative)tileFrame=stream.nativeTiles;
    if(tiles.length&&!reuseNative) {
      // Bound the compositor crop to visible tiles; keep protection geometry in
      // full canonical pixels so masks cannot shift with the crop origin.
      const scale=this.host.scales.get(tab.tab_id)??1;
      const x0=Math.max(0,Math.floor(Math.min(...tiles.map(n=>tileBoxes.get(n.id).x)))),y0=Math.max(0,Math.floor(Math.min(...tiles.map(n=>tileBoxes.get(n.id).y))));
      const x1=Math.min(1280,Math.ceil(Math.max(...tiles.map(n=>tileBoxes.get(n.id).x+n.box.width)))),y1=Math.min(800,Math.ceil(Math.max(...tiles.map(n=>tileBoxes.get(n.id).y+n.box.height))));
      // MP-08/MP-10: motion uses the negotiated PNG crop; the next idle
      // credit performs native full-frame verification/refinement, as display
      // does. Chromium's DPR2 cropped raster can differ at glyph/shape edges.
      const inputEpoch=this.host.inputEpochs?.get(tab.tab_id)??0;
      const refine=stream.refinePending&&stream.tileRevision===sourceRevision&&stream.tileInputEpoch===inputEpoch&&stream.nativeRasterKey===nativeRasterKey;
      const clip=refine?{x:0,y:0,width:1280,height:800,scale:1}:{x:x0,y:y0,width:x1-x0,height:y1-y0,scale:1};
      const masks=await captureRegionMasks(world.connection,world.sessionId,{mirrorStructured:true});
      mark('tile_masks_before');
      const captured=await this.host.screenshot(tab,refine?null:clip);
      stream.refinePending=!refine;stream.tileRevision=sourceRevision;stream.tileInputEpoch=inputEpoch;
      mark('tile_capture');
      const protectedRegions=await masks.afterCapture({width:1280*scale,height:800*scale});
      mark('tile_masks_after');
      const frame=maskPixels(decodePng(captured.data_base64,scale),protectedRegions.map(r=>[r.x-clip.x*scale,r.y-clip.y*scale,r.width,r.height]));tileFrame=[];
      for(const node of tiles) {
        const box=tileBoxes.get(node.id);
        const x=Math.max(0,Math.floor(box.x-clip.x)*scale),y=Math.max(0,Math.floor(box.y-clip.y)*scale);
        const w=Math.max(0,Math.min(frame.width,Math.ceil(box.x+box.width-clip.x)*scale)-x),h=Math.max(0,Math.min(frame.height,Math.ceil(box.y+box.height-clip.y)*scale)-y);
        if(!w||!h)continue;
        const exact=losslessRegion(frame,x,y,w,h);
        tileFrame.push({node_id:node.id,x:x/scale+clip.x-box.x,y:y/scale+clip.y-box.y,width:w/scale,height:h/scale,data_base64:exact.data_base64});
      }
    }
    if(tiles.length&&!reuseNative){stream.nativeRasterKey=nativeRasterKey;stream.nativeRasterPolicy=policy;stream.nativeTiles=tileFrame;stream.nativeSettled=nativeOnly&&!animating&&!stream.refinePending}
    mark('tile_decode_mask_encode');
    await assertCurrentDocument(world.connection,world.sessionId,tab.target_id,tab.document_id);assertNotCancelled(signal);
    // MP-11: protection updates may interleave with awaited CDP/resource work.
    // No packet or private base captured under an old policy may escape afterward.
    const assertPolicy=()=>{if(this.host.protection!==policy||this.host.generation!==command.generation||this.streams.get(command.subscription_id)!==stream) {
      stream.previous=null;stream.observed=null;stream.policy=null;stream.geometryResetPolicy=null;stream.epochs=[];stream.refinePending=false;stream.resources.clear();stream.cache.clear();
      throw new Error('MP-11: stale mirror protection policy or subscription');
    }}
    assertPolicy();
    const custom=stream.frameCustom;await custom.verify();stream.inputCustomFingerprint=custom.fingerprint;
    if(sourceEpoch!==null&&await this.evaluate(world,'globalThis.__charioxMirror.epoch()')!==sourceEpoch){assertPolicy();throw new MirrorFrameChanged();}
    stream.frameWorld=world;stream.frameSourceRevision=sourceEpoch;
    mark('document_fence');assertPolicy();
    const hash=stream.hasher.hash(source),reset=!stream.previous||command.after_sequence!==stream.sequence||stream.document_id!==tab.document_id;
    const previous=new Map((stream.previous?.nodes??[]).map(n=>[n.id,n]));
    const changed=reset?source.nodes:source.nodes.filter(n=>JSON.stringify(n)!==JSON.stringify(previous.get(n.id)));
    const removed=reset?[]:[...previous.keys()].filter(id=>!byId.has(id)||hidden.has(id));
    const resources=[...material.resources.values()].filter(r=>reset||!stream.resources.has(r.resource_id));
    const packet={subscription_id:command.subscription_id,tab_id:tab.tab_id,generation:command.generation,document_id:tab.document_id,sequence:stream.sequence+1,base_sequence:reset?null:stream.sequence,reset,hash,root:source.root,nodes:changed,removed,fonts:source.fonts,scroll:source.scroll,focused:source.focused,selection:source.selection??null,resources,tiles:tileFrame??[],css_width:1280,css_height:800,device_scale_factor:this.host.scales.get(tab.tab_id)??1};
    if(JSON.stringify(packet).length>maxWire)throw new Error('MP-11: mirror packet exceeds bound; use display fallback');
    stream.geometryResetPolicy=null;stream.sequence++;stream.previous=source;stream.document_id=tab.document_id;stream.policy=policy;stream.resources=material.resources;
    // MP-11: only committed/issued epochs are eligible; no future or guessed input.
    stream.epochs.push({sequence:stream.sequence,issuedAt:this.now(),nodes:new Map(source.nodes.map(n=>[n.id,JSON.stringify(n)])),focused:source.focused,fullFallback:stream.fullFallback});
    stream.epochs=stream.epochs.filter(e=>this.now()-e.issuedAt<=2000).slice(-8);
    mark('hash_diff_serialize');this.host.timing?.('mirror_total',started);
    return packet;
  }
  async resolveInput(tab,input,scope,signal) {
    this.assertWebTab(tab);const stream=this.require(input.subscription_id,scope,this.host.generation);
    if(stream.tab_id!==tab.tab_id||stream.document_id!==tab.document_id)throw new Error('MP-11: stale mirror input document');
    if(stream.policy===null&&stream.geometryResetPolicy===this.host.protection)throw new MirrorInputEpochRefusal();
    if(stream.policy!==this.host.protection)throw new Error('MP-11: stale mirror protection policy');
    stream.epochs=stream.epochs.filter(e=>this.now()-e.issuedAt<=2000).slice(-8);
    if(stream.pending?.sequence===input.sequence&&!stream.pending.lastIssued)throw new MirrorInputEpochRefusal();
    const epoch=stream.epochs.find(e=>e.sequence===input.sequence);
    // This marker means admission refused before any CDP/input work. A
    // wheel may refresh changed viewport geometry here; clicks never replay.
    if(!epoch)throw new MirrorInputEpochRefusal();
    const generation=this.host.generation;
    const assertEpoch=()=>{if(this.require(input.subscription_id,scope,generation)!==stream||stream.policy!==this.host.protection||stream.document_id!==tab.document_id||!stream.epochs.includes(epoch)||this.now()-epoch.issuedAt>2000){if(stream.policy===null&&stream.geometryResetPolicy===this.host.protection)throw new MirrorInputEpochRefusal();throw new Error('MP-11: stale mirror protection policy or admitted input');}};
    const action=input.action;
    if(stream.fullFallback && !['coordinate','key'].includes(action?.kind))throw new Error('MP-11: full video fallback requires coordinate input');
    if(!action||typeof action.kind!=='string'||['text','composition'].includes(action.kind)&&(typeof action.text!=='string'||action.text.length>16384)||action.kind==='composition'&&(!Number.isInteger(action.selection_start)||!Number.isInteger(action.selection_end)||action.selection_start<0||action.selection_start>action.text.length||action.selection_end<action.selection_start||action.selection_end>action.text.length))throw new Error('MP-11: invalid mirror input');
    const records=new Map(stream.previous.nodes.map(n=>[n.id,n]));
    for(const id of action.kind==='selection'?[action.anchor_id,action.focus_id]:action.kind==='key'||action.kind==='coordinate'?[]:[action.node_id]) {
      const record=records.get(id);if(!record||record.kind==='mask'||action.kind==='selection'&&record.kind!=='text')throw new Error('MP-11: protected or unknown mirror input');
    }
    const unchanged=id=>{
      const record=records.get(id),old=epoch.nodes.has(id)?JSON.parse(epoch.nodes.get(id)):null;
      if(!record||!old||record.kind==='mask'||record.kind!==old.kind||record.tag!==old.tag||record.parent!==old.parent)return false;
      // Printable edits may change a field's value without replacing its target.
      // Selection offsets and pointer geometry must still refer to the old view.
      if(action.kind==='selection'&&record.text!==old.text)return false;
      if(['frame','document'].includes(record.kind)&&JSON.stringify(record.children)!==JSON.stringify(old.children))return false;
      if(['click','coordinate'].includes(action.kind)&&JSON.stringify(record.box)!==JSON.stringify(old.box))return false;
      return JSON.stringify(record.attributes??{})===JSON.stringify(old.attributes??{});
    };
    const targets=action.kind==='selection'?[action.anchor_id,action.focus_id]:['key','coordinate'].includes(action.kind)?[]:[action.node_id];
    if(targets.some(id=>!unchanged(id)))throw new Error('MP-11: changed or unknown mirror input target');
    if(action.kind==='selection')for(const [id,offset]of [[action.anchor_id,action.anchor_offset],[action.focus_id,action.focus_offset]])if(!Number.isInteger(offset)||offset<0||offset>(records.get(id)?.text?.length??0))throw new Error('MP-11: invalid mirror selection endpoint');
    if(action.kind==='key'&&(epoch.focused!==stream.previous.focused||epoch.focused&&!unchanged(epoch.focused)))throw new Error('MP-11: changed mirror focus');
    let coordinateExpected=[];
    if(action.kind==='coordinate') {
      if(epoch.fullFallback!==stream.fullFallback)throw new Error('MP-11: changed mirror coordinate surface');
      const point=action.input;
      const changed=message=>{if(point?.kind==='scroll')throw new MirrorInputEpochRefusal();throw new Error(message);};
      const candidates=new Set();
      if(point?.kind==='click'||point?.kind==='scroll')for(const record of records.values())if(record.box&&['element','frame','tile','mask'].includes(record.kind)) {
        const box={...record.box};for(let parent=records.get(record.parent);parent;parent=records.get(parent.parent))if(parent.kind==='frame'){box.x+=parent.box.x+(parseFloat(parent.style?.['border-left-width'])||0)+(parseFloat(parent.style?.['padding-left'])||0);box.y+=parent.box.y+(parseFloat(parent.style?.['border-top-width'])||0)+(parseFloat(parent.style?.['padding-top'])||0);}
        if(point.x>=box.x&&point.x<box.x+box.width&&point.y>=box.y&&point.y<box.y+box.height){if(!unchanged(record.id))changed('MP-11: changed mirror coordinate target');candidates.add(record.id);}
      }
      if(!stream.fullFallback&&['click','scroll'].includes(point?.kind)) {
        // MP-11: bind coordinates to one observed leaf, never whichever live
        // sibling now occupies the point. Ambiguous overlapping leaves refuse.
        const ancestors=new Set();for(const id of candidates)for(let node=records.get(records.get(id).parent);node;node=records.get(node.parent))ancestors.add(node.id);
        const leaves=[...candidates].filter(id=>!ancestors.has(id));
        if(leaves.length!==1)throw new Error('MP-11: ambiguous or unknown mirror coordinate target');
        for(let node=records.get(leaves[0]);node;node=records.get(node.parent))if(['element','frame','tile','mask'].includes(node.kind)) {
          if(!unchanged(node.id))changed('MP-11: changed mirror coordinate ancestor');
          const old=JSON.parse(epoch.nodes.get(node.id));coordinateExpected.push({id:old.id,kind:old.kind,box:old.box,attributes:old.attributes});
        }
      }
    }
    const world=await this.world(tab);
    const custom=await inspectMirrorCustomElements(world);
    try{await custom.verify();if(custom.fingerprint!==stream.inputCustomFingerprint)throw Error('MP-11: changed custom host input epoch')}finally{await custom.release()}assertNotCancelled(signal);assertEpoch();
    const call=async method=>{assertNotCancelled(signal);assertEpoch();await assertCurrentDocument(world.connection,world.sessionId,tab.target_id,tab.document_id);assertEpoch();const result=await this.evaluate(world,`(()=>{globalThis.__charioxMirror.validate(${JSON.stringify(targets.map(id=>records.get(id)))});return globalThis.__charioxMirror.${method}(${JSON.stringify(action)})})()`);assertEpoch();return result;};
    if(action.kind==='selection')return {perform:(_send,mark)=>{mark?.();return call('select');}};
    if(action.kind==='focus')return {perform:(_send,mark)=>{mark?.();return call('focus');}};
    if(action.kind==='coordinate') {
      if(!stream.fullFallback&&['text','key'].includes(action.input?.kind)) {
        // MP-08/MP-11: following native input, resolve native focus at dispatch.
        // Never refocus the old viewer field. Unknown/protected/new focus fails.
        // A Tab keyDown may move focus; its keyUp must stay paired.
        const editable=action.input.kind==='text';let checked=false;
        const guard=async()=>{
          assertEpoch();assertNotCancelled(signal);if(checked)return;
          const id=await this.evaluate(world,`globalThis.__charioxMirror.activeTarget([],${editable})`);assertEpoch();
          if(!unchanged(id))throw new Error('MP-11: changed native mirror text focus');
          const expected=[];for(let node=records.get(id);node;node=records.get(node.parent))if(['element','frame','tile','mask'].includes(node.kind)) {
            if(!unchanged(node.id))throw new Error('MP-11: changed native mirror text ancestor');
            const old=JSON.parse(epoch.nodes.get(node.id));expected.push({id:old.id,kind:old.kind,box:old.box,attributes:old.attributes});
          }
          const focused=await this.evaluate(world,`globalThis.__charioxMirror.activeTarget(${JSON.stringify(expected)},${editable})`);assertEpoch();assertNotCancelled(signal);
          if(focused!==id)throw new Error('MP-11: changed native mirror text focus');
          checked=true;
        };
        if(!editable)return {input:action.input,guard};
        if(typeof action.input.text!=='string'||action.input.text.length>16384)throw new Error('MP-11: invalid native mirror text');
        return {guard,perform:send=>send('Input.insertText',{text:action.input.text})};
      }
      let checked=false;
      const guard=async()=>{
        assertEpoch();
        // MP-11: validate after focus emulation and the final document wait,
        // immediately before the first physical pointer event. Release stays
        // paired with press even when the page reacts by moving its controls.
        if(!checked&&coordinateExpected.length) {
          const id=await this.evaluate(world,`globalThis.__charioxMirror.coordinateTarget(${JSON.stringify(action.input)},${JSON.stringify([...records.values()].filter(n=>n.kind==='tile').map(n=>n.id))},${JSON.stringify(coordinateExpected)})`);
          assertNotCancelled(signal);assertEpoch();
          if(action.input.kind==='scroll'&&id?.scroll_epoch_refused===true)throw new MirrorInputEpochRefusal();
          if(id!==coordinateExpected[0].id||!unchanged(id))throw new Error('MP-11: changed live mirror coordinate target');
          checked=true;
        }
      };
      return {input:action.input,guard};
    }
    if(action.kind==='key')return {input:{kind:'key',key:action.key},guard:assertEpoch};
    const point=await call('locate');
    if(action.kind==='click')return {input:{kind:'click',...point},guard:assertEpoch};
    if(action.kind==='scroll')return {input:{kind:'scroll',...point,delta_x:action.delta_x,delta_y:action.delta_y},guard:assertEpoch};
    if(action.kind==='text'||action.kind==='composition')return {perform:async (send,mark)=>{
      mark?.();await call('focus');assertNotCancelled(signal);assertEpoch();
      if(action.kind==='text')return send('Input.insertText',{text:action.text});
      return send('Input.imeSetComposition',{text:action.text,selectionStart:action.selection_start,selectionEnd:action.selection_end});
    }};
    throw new Error('MP-08: unsupported mirror action');
  }
}
