// MP-08/MP-10/MP-11: bounded, caller/document/policy-bound mirroring service.
import {displayDeviceMetrics} from './kernel-browser-geometry.mjs';
import { losslessRegion } from './kernel-browser-display.mjs';
import { timestamp } from './kernel-browser-timing.mjs';
import { randomUUID } from 'node:crypto';
import { mirrorInitialStyles } from './kernel-browser-mirror-styles.mjs';
import { mirrorObserverExpression } from './kernel-browser-mirror-observer.mjs';
import { materializeMirrorResources,MirrorTreeHasher } from './kernel-browser-mirror-resources.mjs';
import { observationProtectedVariants } from './browser-controller-snapshot.mjs';
import { locateBrowserRegions } from './browser-observation-regions.mjs';
import { assertCurrentDocument,assertNotCancelled } from './browser-controller-actions.mjs';
import { captureRegionMasks } from './kernel-browser-region-protection.mjs';
import { decodePng,maskPixels } from './kernel-browser-pixels.mjs';

const lifetime=60000,maxWire=4*1024*1024;
const videoSnapshot=()=>({root:'n9007199254740991',nodes:[{id:'n9007199254740991',parent:null,children:['n9007199254740988','n9007199254740990'],kind:'element',tag:'html',style:{margin:'0px'}},{id:'n9007199254740988',parent:'n9007199254740991',children:[],kind:'element',tag:'head'},{id:'n9007199254740990',parent:'n9007199254740991',children:['n9007199254740989'],kind:'element',tag:'body',style:{margin:'0px'}},{id:'n9007199254740989',parent:'n9007199254740990',children:[],kind:'tile',tag:'div',box:{x:0,y:0,width:1280,height:800},reason:'observer_bounds_or_unavailable'}],resources:[],fonts:[],scroll:{x:0,y:0},focused:null,selection:null});
// Trusted admission error: never constructed from page/CDP error strings.
export class MirrorInputEpochRefusal extends Error {
  constructor() { super('MP-11: stale mirror input epoch'); }
}
export class MirrorService {
  constructor(host) {this.host=host;this.now=()=>performance.now();this.streams=new Map();this.expiry=setInterval(()=>this.expire(),5000);this.expiry.unref?.();}
  invalidate() {for(const stream of this.streams.values()){stream.previous=null;stream.observed=null;stream.resources.clear();stream.cache.clear();stream.policy=null;stream.epochs=[];stream.refinePending=false;}}
  clear() {this.streams.clear();}
  removeTab(tabId) {for(const [id,s] of this.streams)if(s.tab_id===tabId)this.streams.delete(id);}
  expire() {for(const [id,s] of this.streams)if(Date.now()>s.expires)this.streams.delete(id);}
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
    await connection.send('Emulation.setDeviceMetricsOverride',displayDeviceMetrics(1280,800,command.device_scale_factor),sessionId);
    this.host.scales.set(tab.tab_id,command.device_scale_factor);
    const subscription_id=`host-mirror-${randomUUID()}`;
    this.streams.set(subscription_id,{scope,tab_id:tab.tab_id,sequence:0,epochs:[],previous:null,resources:new Map(),cache:new Map(),hasher:new MirrorTreeHasher(),fallback:new Set(),expires:Date.now()+lifetime,policy:null});
    return {subscription_id,generation:this.host.generation,tab_id:tab.tab_id,device_scale_factor:command.device_scale_factor};
  }
  async next(command,scope,options={}) {
    const stream=this.require(command.subscription_id,scope,command.generation);
    if(stream.busy)throw new Error('MP-11: mirror credit already outstanding');
    stream.busy=true;
    try{return await this.readPacket(command,scope,options);}finally{stream.busy=false;}
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
    // MP-08/MP-11: use the screenshot collector's live field identity/type check.
    // Bind nodes in the mirror world rather than comparing different coordinate spaces.
    const targets=policy.targets.filter(t=>t.kind==='browser'&&t.target_id===tab.target_id);
    await this.evaluate(world,'globalThis.__charioxMirror.resetFillTargets()');
    if(targets.length||this.host.browser.fillTargets?.size)await locateBrowserRegions(targets,this.host.browser,policy.values,{contentTarget:tab.target_id,contentScale:this.host.scales.get(tab.tab_id)??1,onPlainField:async field=>{
      // Foreign renderer descendants are compositor tiles, not mirrored nodes.
      if(field.sessionId!==world.sessionId)return;
      const {object}=await world.connection.send('DOM.resolveNode',{backendNodeId:field.backendNodeId,executionContextId:world.contextId},world.sessionId);
      if(!object?.objectId)throw Error('MP-11: mirror fill target unavailable');
      try {
        const reply=await world.connection.send('Runtime.callFunctionOn',{objectId:object.objectId,returnByValue:true,functionDeclaration:'function(){return globalThis.__charioxMirror.addFillTarget(this)}'},world.sessionId);
        if(reply.exceptionDetails||reply.result?.value!==true)throw Error('MP-11: mirror fill target unavailable');
      }finally{await world.connection.send('Runtime.releaseObject',{objectId:object.objectId},world.sessionId).catch(()=>{});}
    }});
    mark('regions');
    let source;stream.fullFallback=false;
    try {source=await this.evaluate(world,`globalThis.__charioxMirror.read([],[],${JSON.stringify(command.subscription_id)},${!stream.observed})`);}catch {
      await assertCurrentDocument(world.connection,world.sessionId,tab.target_id,tab.document_id);
      // Bounded/unsupported DOM becomes the existing protected full video region.
      // Synthetic tile IDs never authorize element input into the original page.
      stream.fullFallback=true;
      source=videoSnapshot();
    }
    if(source.styles) {
      if(!Array.isArray(source.styles)||source.styles.length>12000)throw new Error('MP-11: invalid private CSS palette');
      source.nodes=source.nodes.map(node=>{if(node.style_index===undefined)return node;const {style_index,...record}=node;if(!Number.isSafeInteger(style_index)||!source.styles[style_index])throw new Error('MP-11: invalid private CSS reference');return {...record,style:source.styles[style_index]};});
      delete source.styles;
    }
    if(source.incremental) {
      if(!stream.observed)throw new Error('MP-11: mirror observer lost base');
      const records=new Map(stream.observed.nodes.map(n=>[n.id,n]));
      for(const id of source.removed)records.delete(id);
      for(const n of source.nodes)records.set(n.id,n);
      const ordered=[],visit=id=>{const n=records.get(id);if(!n||ordered.length>=12000)throw new Error('MP-11: invalid observer delta');ordered.push(n);for(const child of n.children)visit(child);};visit(source.root);
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
    const material=await materializeMirrorResources(world.connection,world.sessionId,source.resources,policy.values,stream.cache);
    mark('resources');
    for(const node of source.nodes) {
      if(node.resource) {node.resource=material.mapped.get(node.resource)??undefined;if(!node.resource){node.kind='tile';node.tag='img';node.reason='resource_unavailable';}}
      for(const [key,value] of Object.entries(node.style??{})) if(value.startsWith('resource:')) {
        const rid=material.mapped.get(value.slice(9));if(rid)node.style[key]=`resource:${rid}`;else {node.style[key]='none';node.kind='tile';node.tag='img';node.reason='resource_unavailable';}
      }
      if(stream.fallback.has(node.id)&&node.kind==='element'){node.kind='tile';node.tag='img';node.reason='layout_drift';}
    }
    const unavailableFonts=source.fonts.filter(f=>!material.mapped.get(f.resource)).map(f=>f.family.replaceAll('"','').replaceAll("'",'').trim().toLowerCase());
    for(const node of source.nodes)if(node.kind==='element'&&unavailableFonts.some(f=>(node.style?.['font-family']??'').split(',').some(value=>value.replaceAll('"','').replaceAll("'",'').trim().toLowerCase()===f))){node.kind='tile';node.tag='img';node.reason='font_unavailable';}
    source.fonts=source.fonts.flatMap(f=>{const resource=material.mapped.get(f.resource);return resource?[{...f,resource}]:[];});
    const sourceRevision=source.revision??0;
    delete source.resources;delete source.revision;source.selection??=null;
    const compositingNodes=new Map(source.nodes.map(n=>[n.id,n]));
    const unsupportedTile=source.nodes.some(n=>{if(n.kind!=='tile')return false;for(let e=n;e;e=compositingNodes.get(e.parent)){const style=e.style??{};if(['transform','filter','backdrop-filter','perspective'].some(key=>style[key]&&style[key]!=='none')||style.opacity&&style.opacity!=='1')return true;}return false;});
    if(unsupportedTile){stream.fullFallback=true;source=videoSnapshot();delete source.resources;}
    if(['tile','mask'].includes(source.nodes.find(n=>n.id===source.root)?.kind)){stream.fullFallback=true;source=videoSnapshot();delete source.resources;}
    // A tile is an opaque subtree. Descendants must not remain in the wire/map.
    const byId=new Map(source.nodes.map(n=>[n.id,n])),hidden=new Set();
    const hide=id=>{hidden.add(id);for(const child of byId.get(id)?.children??[])hide(child);};
    for(const n of source.nodes)if(n.kind==='tile'||n.kind==='mask'){for(const child of n.children)hide(child);n.children=[];}
    source.nodes=source.nodes.filter(n=>!hidden.has(n.id));
    if(source.selection&&!source.nodes.some(n=>n.id===source.selection.anchor_id&&n.kind==='text')||source.selection&&!source.nodes.some(n=>n.id===source.selection.focus_id&&n.kind==='text'))source.selection=null;
    if(source.focused&&!source.nodes.some(n=>n.id===source.focused&&n.kind!=='mask'))source.focused=null;
    // MP-08/MP-11: opaque subtrees use protected compositor tiles.
    // Ordinary foreign-frame and closed-shadow pixels remain visible.
    const globalBox=node=>{const box={...node.box};for(let ancestor=byId.get(node.parent);ancestor;ancestor=byId.get(ancestor.parent))if(ancestor.kind==='frame') {box.x+=ancestor.box.x+(parseFloat(ancestor.style?.['border-left-width'])||0)+(parseFloat(ancestor.style?.['padding-left'])||0);box.y+=ancestor.box.y+(parseFloat(ancestor.style?.['border-top-width'])||0)+(parseFloat(ancestor.style?.['padding-top'])||0);}return box;};
    const tiles=source.nodes.filter(n=>{if(n.kind!=='tile'||!(n.box?.width>0&&n.box?.height>0))return false;const b=globalBox(n);return b.x<1280&&b.y<800&&b.x+b.width>0&&b.y+b.height>0;});
    const tileBoxes=new Map(tiles.map(n=>[n.id,globalBox(n)]));
    if(tiles.length>64)throw new Error('MP-11: visible tile limit; use display fallback');
    mark('sanitize');
    let tileFrame=null;
    if(tiles.length) {
      // Bound the compositor crop to visible tiles; keep protection geometry in
      // full canonical pixels so masks cannot shift with the crop origin.
      const scale=this.host.scales.get(tab.tab_id)??1;
      const x0=Math.max(0,Math.floor(Math.min(...tiles.map(n=>tileBoxes.get(n.id).x)))),y0=Math.max(0,Math.floor(Math.min(...tiles.map(n=>tileBoxes.get(n.id).y))));
      const x1=Math.min(1280,Math.ceil(Math.max(...tiles.map(n=>tileBoxes.get(n.id).x+n.box.width)))),y1=Math.min(800,Math.ceil(Math.max(...tiles.map(n=>tileBoxes.get(n.id).y+n.box.height))));
      // MP-08/MP-10: motion uses the negotiated PNG crop; the next idle
      // credit performs native full-frame verification/refinement, as display
      // does. Chromium's DPR2 cropped raster can differ at glyph/shape edges.
      const inputEpoch=this.host.inputEpochs?.get(tab.tab_id)??0;
      const refine=stream.refinePending&&stream.tileRevision===sourceRevision&&stream.tileInputEpoch===inputEpoch;
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
    mark('tile_decode_mask_encode');
    await assertCurrentDocument(world.connection,world.sessionId,tab.target_id,tab.document_id);assertNotCancelled(signal);
    mark('document_fence');
    // MP-11: protection updates may interleave with awaited CDP/resource work.
    // No packet or private base captured under an old policy may escape afterward.
    if(this.host.protection!==policy||this.host.generation!==command.generation||this.streams.get(command.subscription_id)!==stream) {
      stream.previous=null;stream.observed=null;stream.policy=null;stream.epochs=[];stream.refinePending=false;stream.resources.clear();stream.cache.clear();
      throw new Error('MP-11: stale mirror protection policy or subscription');
    }
    const hash=stream.hasher.hash(source),reset=!stream.previous||command.after_sequence!==stream.sequence||stream.document_id!==tab.document_id;
    const previous=new Map((stream.previous?.nodes??[]).map(n=>[n.id,n]));
    const changed=reset?source.nodes:source.nodes.filter(n=>JSON.stringify(n)!==JSON.stringify(previous.get(n.id)));
    const removed=reset?[]:[...previous.keys()].filter(id=>!byId.has(id)||hidden.has(id));
    const resources=[...material.resources.values()].filter(r=>reset||!stream.resources.has(r.resource_id));
    const packet={subscription_id:command.subscription_id,tab_id:tab.tab_id,generation:command.generation,document_id:tab.document_id,sequence:stream.sequence+1,base_sequence:reset?null:stream.sequence,reset,hash,root:source.root,nodes:changed,removed,fonts:source.fonts,scroll:source.scroll,focused:source.focused,selection:source.selection??null,resources,tiles:tileFrame??[],css_width:1280,css_height:800,device_scale_factor:this.host.scales.get(tab.tab_id)??1};
    if(JSON.stringify(packet).length>maxWire)throw new Error('MP-11: mirror packet exceeds bound; use display fallback');
    stream.sequence++;stream.previous=source;stream.document_id=tab.document_id;stream.policy=policy;stream.resources=material.resources;
    // MP-11: only committed/issued epochs are eligible; no future or guessed input.
    stream.epochs.push({sequence:stream.sequence,issuedAt:this.now(),nodes:new Map(source.nodes.map(n=>[n.id,JSON.stringify(n)])),focused:source.focused,fullFallback:stream.fullFallback});
    stream.epochs=stream.epochs.filter(e=>this.now()-e.issuedAt<=2000).slice(-8);
    mark('hash_diff_serialize');this.host.timing?.('mirror_total',started);
    return packet;
  }
  async resolveInput(tab,input,scope,signal) {
    this.assertWebTab(tab);const stream=this.require(input.subscription_id,scope,this.host.generation);
    if(stream.tab_id!==tab.tab_id||stream.document_id!==tab.document_id)throw new Error('MP-11: stale mirror input document');
    if(stream.policy!==this.host.protection)throw new Error('MP-11: stale mirror protection policy');
    stream.epochs=stream.epochs.filter(e=>this.now()-e.issuedAt<=2000).slice(-8);
    const epoch=stream.epochs.find(e=>e.sequence===input.sequence);
    // This marker ONLY means sequence admission refused before any CDP/input
    // work. A later fence or changed target is never a sequence-only refusal.
    if(!epoch)throw new MirrorInputEpochRefusal();
    const generation=this.host.generation;
    const assertEpoch=()=>{if(this.require(input.subscription_id,scope,generation)!==stream||stream.policy!==this.host.protection||stream.document_id!==tab.document_id||!stream.epochs.includes(epoch)||this.now()-epoch.issuedAt>2000)throw new Error('MP-11: stale mirror protection policy or admitted input');};
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
      const candidates=new Set();
      if(point?.kind==='click'||point?.kind==='scroll')for(const record of records.values())if(record.box&&['element','frame','tile','mask'].includes(record.kind)) {
        const box={...record.box};for(let parent=records.get(record.parent);parent;parent=records.get(parent.parent))if(parent.kind==='frame'){box.x+=parent.box.x+(parseFloat(parent.style?.['border-left-width'])||0)+(parseFloat(parent.style?.['padding-left'])||0);box.y+=parent.box.y+(parseFloat(parent.style?.['border-top-width'])||0)+(parseFloat(parent.style?.['padding-top'])||0);}
        if(point.x>=box.x&&point.x<box.x+box.width&&point.y>=box.y&&point.y<box.y+box.height){if(!unchanged(record.id))throw new Error('MP-11: changed mirror coordinate target');candidates.add(record.id);}
      }
      if(!stream.fullFallback&&['click','scroll'].includes(point?.kind)) {
        // MP-11: bind coordinates to one observed leaf, never whichever live
        // sibling now occupies the point. Ambiguous overlapping leaves refuse.
        const ancestors=new Set();for(const id of candidates)for(let node=records.get(records.get(id).parent);node;node=records.get(node.parent))ancestors.add(node.id);
        const leaves=[...candidates].filter(id=>!ancestors.has(id));
        if(leaves.length!==1)throw new Error('MP-11: ambiguous or unknown mirror coordinate target');
        for(let node=records.get(leaves[0]);node;node=records.get(node.parent))if(['element','frame','tile','mask'].includes(node.kind)) {
          if(!unchanged(node.id))throw new Error('MP-11: changed mirror coordinate ancestor');
          const old=JSON.parse(epoch.nodes.get(node.id));coordinateExpected.push({id:old.id,kind:old.kind,box:old.box,attributes:old.attributes});
        }
      }
    }
    const world=await this.world(tab);assertNotCancelled(signal);assertEpoch();
    const call=async method=>{assertNotCancelled(signal);assertEpoch();await assertCurrentDocument(world.connection,world.sessionId,tab.target_id,tab.document_id);assertEpoch();const result=await this.evaluate(world,`(()=>{globalThis.__charioxMirror.validate(${JSON.stringify(targets.map(id=>records.get(id)))});return globalThis.__charioxMirror.${method}(${JSON.stringify(action)})})()`);assertEpoch();return result;};
    if(action.kind==='selection')return {perform:(_send,mark)=>{mark?.();return call('select');}};
    if(action.kind==='focus')return {perform:(_send,mark)=>{mark?.();return call('focus');}};
    if(action.kind==='key'||action.kind==='coordinate') {
      const physical=action.kind==='key'?{kind:'key',key:action.key}:action.input;
      if(!stream.fullFallback&&['text','key'].includes(physical?.kind)) {
        // MP-08/MP-11: bind plain keys to painted focus; coordinate-wrapped
        // input follows native focus after Tab. Validate both at dispatch.
        // Never refocus the old viewer field. Unknown/protected/new focus fails.
        // A Tab keyDown may move focus; its keyUp must stay paired.
        const editable=physical.kind==='text';let checked=false;
        const guard=async()=>{
          assertEpoch();assertNotCancelled(signal);if(checked)return;
          const id=await this.evaluate(world,`globalThis.__charioxMirror.activeTarget([],${editable})`);assertEpoch();
          if(!unchanged(id)||action.kind==='key'&&id!==epoch.focused)throw new Error('MP-11: changed native mirror text focus');
          const expected=[];for(let node=records.get(id);node;node=records.get(node.parent))if(['element','frame','tile','mask'].includes(node.kind)) {
            if(!unchanged(node.id))throw new Error('MP-11: changed native mirror text ancestor');
            const old=JSON.parse(epoch.nodes.get(node.id));expected.push({id:old.id,kind:old.kind,box:old.box,attributes:old.attributes});
          }
          const focused=await this.evaluate(world,`globalThis.__charioxMirror.activeTarget(${JSON.stringify(expected)},${editable})`);assertEpoch();assertNotCancelled(signal);
          if(focused!==id)throw new Error('MP-11: changed native mirror text focus');
          checked=true;
        };
        if(!editable)return {input:physical,guard,observedFrameInput:true};
        if(typeof physical.text!=='string'||physical.text.length>16384)throw new Error('MP-11: invalid native mirror text');
        return {guard,observedFrameInput:true,perform:send=>send('Input.insertText',{text:physical.text})};
      }
      if(action.kind==='key')throw new Error('MP-11: mirror key requires observed focus; use coordinate input for display fallback');
      let checked=false;
      const guard=async()=>{
        assertEpoch();
        // MP-11: validate after focus emulation and the final document wait,
        // immediately before the first physical pointer event. Release stays
        // paired with press even when the page reacts by moving its controls.
        if(!checked&&coordinateExpected.length) {
          const id=await this.evaluate(world,`globalThis.__charioxMirror.coordinateTarget(${JSON.stringify(action.input)},${JSON.stringify([...records.values()].filter(n=>n.kind==='tile').map(n=>n.id))},${JSON.stringify(coordinateExpected)})`);
          assertNotCancelled(signal);assertEpoch();
          if(id!==coordinateExpected[0].id||!unchanged(id))throw new Error('MP-11: changed live mirror coordinate target');
          checked=true;
        }
      };
      return {input:action.input,guard};
    }
    const point=await call('locate');
    // MP-11: bind addressed actions to the observed target and ancestry, then
    // revalidate in one live evaluation after native preparation/preflights.
    const expected=[];
    if(['click','text','composition'].includes(action.kind))for(let node=records.get(action.node_id);node;node=records.get(node.parent))if(['element','frame','tile','mask'].includes(node.kind)) {
      if(!unchanged(node.id))throw new Error('MP-11: changed mirror input ancestor');
      const old=JSON.parse(epoch.nodes.get(node.id));expected.push({id:old.id,kind:old.kind,box:old.box,attributes:old.attributes});
    }
    if(action.kind==='click') {
      let checked=false;
      const guard=async()=>{
        assertEpoch();assertNotCancelled(signal);if(checked)return;
        const id=await this.evaluate(world,`globalThis.__charioxMirror.coordinateTarget(${JSON.stringify(point)},[],${JSON.stringify(expected)},true)`);
        assertEpoch();assertNotCancelled(signal);
        if(id!==action.node_id)throw new Error('MP-11: changed live mirror click target');
        // A press can change the page; release must remain paired.
        checked=true;
      };
      return {input:{kind:'click',...point},guard};
    }
    if(action.kind==='scroll')return {input:{kind:'scroll',...point,delta_x:action.delta_x,delta_y:action.delta_y},guard:assertEpoch};
    if(action.kind==='text'||action.kind==='composition')return {observedFrameInput:true,guard:async()=>{
      assertEpoch();assertNotCancelled(signal);
      const focused=await this.evaluate(world,`globalThis.__charioxMirror.activeTarget(${JSON.stringify(expected)})`);
      assertEpoch();assertNotCancelled(signal);
      if(focused!==action.node_id)throw new Error('MP-11: changed live mirror text focus');
    },perform:async (send,mark)=>{
      mark?.();await call('focus');assertNotCancelled(signal);assertEpoch();
      if(action.kind==='text')return send('Input.insertText',{text:action.text});
      return send('Input.imeSetComposition',{text:action.text,selectionStart:action.selection_start,selectionEnd:action.selection_end});
    }};
    throw new Error('MP-08: unsupported mirror action');
  }
}
