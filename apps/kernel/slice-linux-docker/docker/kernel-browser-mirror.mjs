// MP-08/MP-10/MP-11: bounded, caller/document/policy-bound mirroring service.
import { randomUUID } from 'node:crypto';
import { MIRROR_OBSERVER_EXPRESSION } from './kernel-browser-mirror-observer.mjs';
import { materializeMirrorResources,mirrorHash } from './kernel-browser-mirror-resources.mjs';
import { observationProtectedVariants } from './browser-controller-snapshot.mjs';
import { locateBrowserRegions } from './browser-observation-regions.mjs';
import { assertCurrentDocument,assertNotCancelled } from './browser-controller-actions.mjs';
import { captureRegionMasks } from './kernel-browser-region-protection.mjs';
import { decodePng,encodePng,maskPng } from './kernel-browser-pixels.mjs';

const lifetime=60000,maxWire=4*1024*1024;
const videoSnapshot=()=>({root:'n9007199254740991',nodes:[{id:'n9007199254740991',parent:null,children:['n9007199254740990'],kind:'element',tag:'html',style:{margin:'0px'}},{id:'n9007199254740990',parent:'n9007199254740991',children:['n9007199254740989'],kind:'element',tag:'body',style:{margin:'0px'}},{id:'n9007199254740989',parent:'n9007199254740990',children:[],kind:'tile',tag:'div',box:{x:0,y:0,width:1280,height:800},reason:'observer_bounds_or_unavailable'}],resources:[],fonts:[],scroll:{x:0,y:0},focused:null,selection:null});
export class MirrorService {
  constructor(host) {this.host=host;this.streams=new Map();this.expiry=setInterval(()=>this.expire(),5000);this.expiry.unref?.();}
  invalidate() {for(const stream of this.streams.values()){stream.previous=null;stream.resources.clear();stream.cache.clear();stream.policy=null;}}
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
    await assertCurrentDocument(connection,sessionId,tab.target_id,tab.document_id);
    const {frameTree}=await connection.send('Page.getFrameTree',{},sessionId);
    const world=await this.host.browser.ensureFocusWorld(connection,sessionId,tab.target_id,frameTree.frame);
    if(!Number.isSafeInteger(world.contextId)||world.contextId<=0) throw new Error('MP-11: mirror isolated world unavailable');
    if(!world.mirrorInstalled) {
      const installed=await connection.send('Runtime.evaluate',{expression:MIRROR_OBSERVER_EXPRESSION,contextId:world.contextId,returnByValue:true},sessionId);
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
    this.streams.set(subscription_id,{scope,tab_id:tab.tab_id,sequence:0,previous:null,resources:new Map(),cache:new Map(),fallback:new Set(),expires:Date.now()+lifetime,policy:null});
    return {subscription_id,generation:this.host.generation,tab_id:tab.tab_id,device_scale_factor:command.device_scale_factor};
  }
  async next(command,scope,{signal}={}) {
    const stream=this.require(command.subscription_id,scope,command.generation),tab=await this.host.displayTarget({tab_id:stream.tab_id,generation:command.generation});
    this.assertWebTab(tab);assertNotCancelled(signal);const world=await this.world(tab),policy=this.host.protection;
    if(stream.document_id!==tab.document_id) {stream.previous=null;stream.resources.clear();stream.cache.clear();stream.fallback.clear();}
    if(stream.policy!==policy) {stream.previous=null;stream.resources.clear();stream.cache.clear();}
    if(!Array.isArray(command.drift_nodes)||command.drift_nodes.length>64||command.drift_nodes.some(id=>!stream.previous?.nodes.some(n=>n.id===id&&n.kind==='element'))) throw new Error('MP-11: invalid drift report');
    for(const id of command.drift_nodes)stream.fallback.add(id);
    // Registered Vault target geometry and plaintext/media echoes use the SAME
    // trusted CDP locator as screenshots. A failed locator fences DOM output.
    const targets=policy.targets.filter(t=>t.kind==='browser'&&t.target_id===tab.target_id);
    const regions=targets.length?await locateBrowserRegions(targets,this.host.browser,policy.values,{contentTarget:tab.target_id,contentScale:this.host.scales.get(tab.tab_id)??1}):[];
    let source;stream.fullFallback=false;
    try {source=await this.evaluate(world,`globalThis.__charioxMirror.read(${JSON.stringify(observationProtectedVariants(policy.values))},${JSON.stringify(regions)})`);}catch {
      await assertCurrentDocument(world.connection,world.sessionId,tab.target_id,tab.document_id);
      // Bounded/unsupported DOM becomes the existing protected full video region.
      // Synthetic tile IDs never authorize element input into the original page.
      stream.fullFallback=true;
      source=videoSnapshot();
    }
    // Observer supplies only sanitized content; original resource URLs remain private.
    const material=await materializeMirrorResources(world.connection,world.sessionId,source.resources,policy.values,stream.cache);
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
    const tiles=source.nodes.filter(n=>n.kind==='tile'&&n.box?.width>0&&n.box?.height>0&&n.box.x<1280&&n.box.y<800&&n.box.x+n.box.width>0&&n.box.y+n.box.height>0);
    if(tiles.length>64)throw new Error('MP-11: visible tile limit; use display fallback');
    let tileFrame=null;
    if(tiles.length) {
      // Bound the compositor crop to visible tiles; keep protection geometry in
      // full canonical pixels so masks cannot shift with the crop origin.
      const scale=this.host.scales.get(tab.tab_id)??1;
      const x0=Math.max(0,Math.floor(Math.min(...tiles.map(n=>n.box.x)))),y0=Math.max(0,Math.floor(Math.min(...tiles.map(n=>n.box.y))));
      const x1=Math.min(1280,Math.ceil(Math.max(...tiles.map(n=>n.box.x+n.box.width)))),y1=Math.min(800,Math.ceil(Math.max(...tiles.map(n=>n.box.y+n.box.height))));
      const clip={x:x0,y:y0,width:x1-x0,height:y1-y0,scale:1};
      const masks=await captureRegionMasks(world.connection,world.sessionId,{mirrorStructured:true});
      const captured=await this.host.screenshot(tab,clip);
      const protectedRegions=await masks.afterCapture({width:1280*scale,height:800*scale});
      const frame=decodePng(maskPng(captured.data_base64,protectedRegions.map(r=>[r.x-clip.x*scale,r.y-clip.y*scale,r.width,r.height]),scale),scale);tileFrame=[];
      for(const node of tiles) {
        const x=Math.max(0,Math.floor((node.box.x-clip.x)*scale)),y=Math.max(0,Math.floor((node.box.y-clip.y)*scale));
        const w=Math.max(0,Math.min(frame.width,Math.ceil((node.box.x+node.box.width-clip.x)*scale))-x),h=Math.max(0,Math.min(frame.height,Math.ceil((node.box.y+node.box.height-clip.y)*scale))-y);
        if(!w||!h)continue;
        const pixels=Buffer.alloc(w*h*4);
        for(let row=0;row<h;row++)frame.pixels.copy(pixels,row*w*4,((y+row)*frame.width+x)*4,((y+row)*frame.width+x+w)*4);
        tileFrame.push({node_id:node.id,x:x/scale+clip.x-node.box.x,y:y/scale+clip.y-node.box.y,width:w/scale,height:h/scale,data_base64:encodePng(w,h,pixels)});
      }
    }
    await assertCurrentDocument(world.connection,world.sessionId,tab.target_id,tab.document_id);assertNotCancelled(signal);
    const hash=mirrorHash(source),reset=!stream.previous||command.after_sequence!==stream.sequence||stream.document_id!==tab.document_id;
    const previous=new Map((stream.previous?.nodes??[]).map(n=>[n.id,n]));
    const changed=reset?source.nodes:source.nodes.filter(n=>JSON.stringify(n)!==JSON.stringify(previous.get(n.id)));
    const removed=reset?[]:[...previous.keys()].filter(id=>!byId.has(id)||hidden.has(id));
    const resources=[...material.resources.values()].filter(r=>reset||!stream.resources.has(r.resource_id));
    const packet={subscription_id:command.subscription_id,tab_id:tab.tab_id,generation:command.generation,document_id:tab.document_id,sequence:stream.sequence+1,base_sequence:reset?null:stream.sequence,reset,hash,root:source.root,nodes:changed,removed,fonts:source.fonts,scroll:source.scroll,focused:source.focused,selection:source.selection??null,resources,tiles:tileFrame??[],css_width:1280,css_height:800,device_scale_factor:this.host.scales.get(tab.tab_id)??1};
    if(JSON.stringify(packet).length>maxWire)throw new Error('MP-11: mirror packet exceeds bound; use display fallback');
    stream.sequence++;stream.previous=source;stream.document_id=tab.document_id;stream.policy=policy;stream.resources=material.resources;
    return packet;
  }
  async resolveInput(tab,input,scope,signal) {
    this.assertWebTab(tab);const stream=this.require(input.subscription_id,scope,this.host.generation);
    if(stream.tab_id!==tab.tab_id||stream.document_id!==tab.document_id||stream.sequence!==input.sequence||stream.policy!==this.host.protection)throw new Error('MP-11: stale mirror input epoch');
    const action=input.action;
    if(stream.fullFallback && !['coordinate','key'].includes(action?.kind))throw new Error('MP-11: full video fallback requires coordinate input');
    if(!action||typeof action.kind!=='string'||['text','composition'].includes(action.kind)&&(typeof action.text!=='string'||action.text.length>16384)||action.kind==='composition'&&(!Number.isInteger(action.selection_start)||!Number.isInteger(action.selection_end)||action.selection_start<0||action.selection_start>action.text.length||action.selection_end<action.selection_start||action.selection_end>action.text.length))throw new Error('MP-11: invalid mirror input');
    const records=new Map(stream.previous.nodes.map(n=>[n.id,n]));
    for(const id of action.kind==='selection'?[action.anchor_id,action.focus_id]:action.kind==='key'||action.kind==='coordinate'?[]:[action.node_id]) {
      const record=records.get(id);if(!record||record.kind==='mask'||action.kind==='selection'&&record.kind!=='text')throw new Error('MP-11: protected or unknown mirror input');
    }
    const world=await this.world(tab);assertNotCancelled(signal);
    const call=async method=>{assertNotCancelled(signal);await assertCurrentDocument(world.connection,world.sessionId,tab.target_id,tab.document_id);return this.evaluate(world,`globalThis.__charioxMirror.${method}(${JSON.stringify(action)})`);};
    if(action.kind==='selection')return {perform:()=>call('select')};
    if(action.kind==='focus')return {perform:()=>call('focus')};
    if(action.kind==='coordinate')return {input:action.input};
    if(action.kind==='key')return {input:{kind:'key',key:action.key}};
    const point=await call('locate');
    if(action.kind==='click')return {input:{kind:'click',...point}};
    if(action.kind==='scroll')return {input:{kind:'scroll',...point,delta_x:action.delta_x,delta_y:action.delta_y}};
    if(action.kind==='text'||action.kind==='composition')return {perform:async send=>{
      await call('focus');
      if(action.kind==='text')return send('Input.insertText',{text:action.text});
      return send('Input.imeSetComposition',{text:action.text,selectionStart:action.selection_start,selectionEnd:action.selection_end});
    }};
    throw new Error('MP-08: unsupported mirror action');
  }
}
