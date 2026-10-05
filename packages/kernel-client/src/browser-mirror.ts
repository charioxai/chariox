// MP-08/MP-10/MP-11: reference renderer for Cloud/native web clients. No origin I/O.
import { mirrorSandboxCsp,validateMirrorPacket,mirrorTreeCanonicalJson } from './browser-mirror-security.js'
import type { KernelBrowserMirrorAction, MirrorNode, MirrorPacket, MirrorResource } from './browser-mirror-types.js'
export * from './browser-mirror-types.js'
export { mirrorSandboxCsp } from './browser-mirror-security.js'
export const browserMirrorMinimumProtocolVersion = 433
export interface MirrorTransport { protocolVersion: number; request(request: unknown): Promise<unknown> }
type Binding = { tab_id: string; generation: number; device_scale_factor: 1 | 2 }
async function digest(bytes: Uint8Array): Promise<string> {
  return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes as Uint8Array<ArrayBuffer>)),b=>b.toString(16).padStart(2,'0')).join('')
}
const bytesOf=(data:string):Uint8Array=>Uint8Array.from(atob(data),c=>c.charCodeAt(0))
function blobUrl(data:string,mime:string):string {return URL.createObjectURL(new Blob([bytesOf(data) as Uint8Array<ArrayBuffer>],{type:mime}))}
export class BrowserMirrorRenderer {
  readonly timings:Array<{stage:string;duration_ms:number;started_ms:number;ended_ms:number}>=[]
  private timed(stage:string,at:number):void {this.timings.push({stage,duration_ms:performance.now()-at,started_ms:performance.timeOrigin+at,ended_ms:performance.timeOrigin+performance.now()});if(this.timings.length>2048)this.timings.splice(0,1024)}
  private canonicalNodes=new WeakMap<MirrorNode,string>()
  private records=new Map<string,MirrorNode>()
  private dom=new Map<string,Node>()
  private ids=new WeakMap<Node,string>()
  private resources=new Map<string,string>()
  private tileUrls:string[]=[]
  private overlays:HTMLElement[]=[]
  private dpr=1
  private sequence=0
  private documentId=''
  readonly frame: HTMLIFrameElement
  private doc: Document | null=null
  private loaded:Promise<void>
  private disposed=false
  private applying=false
  private inputChain:Promise<void>=Promise.resolve()
  private removers:Array<()=>void>=[]
  constructor(private container:HTMLElement,private input:(action:KernelBrowserMirrorAction,epoch?:{sequence:number;document_id:string})=>Promise<unknown>,private failure:(error:unknown)=>void) {
    this.frame=container.ownerDocument.createElement('iframe')
    this.frame.setAttribute('sandbox','allow-same-origin') // scripts NEVER enabled
    this.frame.setAttribute('referrerpolicy','no-referrer')
    this.frame.style.cssText='width:1280px;height:800px;border:0;display:block'
    this.frame.srcdoc=`<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${mirrorSandboxCsp}"></head><body></body></html>`
    this.loaded=new Promise<void>(resolve=>this.frame.addEventListener('load',()=>resolve(),{once:true}))
    container.append(this.frame)
  }
  async ready():Promise<void> {
    await this.loaded
    if(this.disposed)throw Error('MP-08: mirror closed')
    this.doc=this.frame.contentDocument
    if(!this.doc)throw Error('MP-11: mirror sandbox unavailable')
    this.bindEvents(this.doc)
  }
  private enqueue(action:KernelBrowserMirrorAction):void {
    if(this.applying||this.disposed)return
    const epoch={sequence:this.sequence,document_id:this.documentId}
    this.inputChain=this.inputChain.then(async()=>{if(!this.disposed)await this.input(action,epoch)}).catch(this.failure)
  }
  private bindEvents(doc:Document):void {
    let lastComposition:string|null=null
    const target=(event:Event):Node=>event.composedPath()[0] as Node
    const point=(event:MouseEvent):{x:number;y:number}=>{let x=event.clientX,y=event.clientY;for(let view:Window|null=doc.defaultView;view&&view!==this.frame.contentWindow;view=view.parent){const frame=view.frameElement as HTMLElement|null;if(!frame)throw Error('MP-11: detached mirror frame');const box=frame.getBoundingClientRect();x+=box.x+frame.clientLeft;y+=box.y+frame.clientTop}return {x:Math.floor(x),y:Math.floor(y)}}
    const id=(node:Node|null):string|undefined=>node ? this.ids.get(node) : undefined
    const on=(kind:string,fn:EventListener):void=>{doc.addEventListener(kind,fn,true);this.removers.push(()=>doc.removeEventListener(kind,fn,true))}
    on('click',event=>{event.preventDefault();const node=id(target(event));if(node){const record=this.records.get(node);if(record?.kind==='mask')return;if(record?.kind==='tile'){const mouse=event as MouseEvent;this.enqueue({kind:'coordinate',input:{kind:'click',...point(mouse)}})}else this.enqueue({kind:'click',node_id:node})}})
    on('wheel',event=>{event.preventDefault();const wheel=event as WheelEvent,node=id(target(event));if(node)this.enqueue({kind:'scroll',node_id:node,delta_x:Math.trunc(wheel.deltaX),delta_y:Math.trunc(wheel.deltaY)})})
    on('keydown',event=>{const key=(event as KeyboardEvent).key;if(['Tab','Enter','Escape','Backspace','Delete','ArrowLeft','ArrowRight','ArrowUp','ArrowDown','Home','End'].includes(key)){event.preventDefault();this.enqueue({kind:'key',key})}})
    on('beforeinput',event=>{const input=event as InputEvent;event.preventDefault();if(input.isComposing||input.inputType.includes('Composition')||input.data===lastComposition)return;const node=id(target(event));if(node&&input.data)this.enqueue({kind:'text',node_id:node,text:input.data})})
    on('compositionupdate',event=>{event.preventDefault();const e=event as CompositionEvent,node=id(target(event));if(node)this.enqueue({kind:'composition',node_id:node,text:e.data,selection_start:e.data.length,selection_end:e.data.length})})
    on('compositionend',event=>{event.preventDefault();const e=event as CompositionEvent,node=id(target(event));if(node){lastComposition=e.data;setTimeout(()=>{lastComposition=null},0);this.enqueue({kind:'text',node_id:node,text:e.data})}})
    on('selectionchange',()=>{if(this.applying)return;const selection=doc.getSelection();if(!selection||selection.isCollapsed)return;const a=id(selection.anchorNode),b=id(selection.focusNode);if(a&&b)this.enqueue({kind:'selection',anchor_id:a,anchor_offset:selection.anchorOffset,focus_id:b,focus_offset:selection.focusOffset})})
  }
  private style(element:HTMLElement,style:Record<string,string>):void {
    element.removeAttribute('style')
    for(const [key,value]of Object.entries(style)) {
      if(key.startsWith('animation')||key.startsWith('transition'))continue // sampled computed state
      const ref=value.startsWith('resource:')?this.resources.get(value.slice(9)):null
      element.style.setProperty(key,value.startsWith('resource:')?(ref?`url("${ref}")`:'none'):value)
    }
  }
  private create(record:MirrorNode,doc:Document):Node {
    let node:Node
    if(record.kind==='text')node=doc.createTextNode(record.text??'')
    else if(record.kind==='document')node=doc.createDocumentFragment()
    else if(record.kind==='shadow')node=doc.createDocumentFragment()
    else node=doc.createElement(record.tag??'div')
    this.dom.set(record.id,node);this.ids.set(node,record.id);return node
  }
  private update(record:MirrorNode,node:Node,previous?:MirrorNode):void {
    if(record.kind==='text'){if(node.textContent!==(record.text??''))node.textContent=record.text??'';return}
    if(node.nodeType!==1)return
    const element=node as HTMLElement
    if(!previous||JSON.stringify(previous.attributes??{})!==JSON.stringify(record.attributes??{})) {
      for(const attr of Array.from(element.attributes))if(attr.name!=='style')element.removeAttribute(attr.name)
      for(const [key,value]of Object.entries(record.attributes??{}))element.setAttribute(key,value)
    }
    // MP-10: child/text changes do not invalidate unchanged sanitized styles.
    if(!previous||JSON.stringify(previous.style??{})!==JSON.stringify(record.style??{}))this.style(element,record.style??{})
    if(record.kind==='mask'){element.style.appearance='none';element.style.borderStyle='solid';element.style.boxShadow='none';element.style.borderRadius='0';if(record.tag==='input'||record.tag==='textarea'){(element as HTMLInputElement).readOnly=true;(element as HTMLInputElement).disabled=true}element.style.background='black';element.style.color='transparent';element.style.borderColor='black';element.setAttribute('aria-label','Protected content')}
    if(record.kind==='tile'||record.kind==='mask') {
      element.style.width=`${record.box?.width??0}px`;element.style.height=`${record.box?.height??0}px`;element.style.position='relative';element.style.overflow='hidden';element.style.background='black'
    }
    if(record.tag==='img'&&record.resource){const url=this.resources.get(record.resource);if(url)(element as HTMLImageElement).src=url}
    if(record.form) {
      const field=element as HTMLInputElement;field.value=record.form.value
      if('checked'in field)field.checked=record.form.checked
      if('selectedIndex'in field)(field as unknown as HTMLSelectElement).selectedIndex=record.form.selected_index
      if(record.form.selection_start!==null&&record.form.selection_end!==null)try{field.setSelectionRange(record.form.selection_start,record.form.selection_end)}catch{}
    }
  }
  async apply(packet:MirrorPacket):Promise<void> {
    const started=performance.now();let at=started
    if(this.disposed||!this.doc)throw Error('MP-08: mirror unavailable')
    if(!packet.reset&&(packet.base_sequence!==this.sequence||packet.document_id!==this.documentId))throw Error('MP-11: mirror lost base')
    const next=validateMirrorPacket(packet,this.records)
    const canonical={root:packet.root,nodes:[...next.values()],fonts:packet.fonts,scroll:packet.scroll,focused:packet.focused,selection:packet.selection}
    // Traversal order can change in a patch; canonicalize by tree order, matching source.
    const order:MirrorNode[]=[];const walk=(id:string):void=>{const n=next.get(id)!;order.push(n);for(const child of n.children)walk(child)};walk(packet.root);canonical.nodes=order
    if(await digest(new TextEncoder().encode(mirrorTreeCanonicalJson(canonical,this.canonicalNodes)))!==packet.hash)throw Error('MP-11: mirror semantic drift')
    this.timed('validate_hash',at);at=performance.now()
    if(packet.reset)this.clearResources()
    for(const resource of packet.resources)await this.addResource(resource)
    const newTiles=packet.tiles.map(tile=>({tile,url:blobUrl(tile.data_base64,'image/png')}))
    this.timed('resource_decode',at);at=performance.now()
    this.dpr=packet.device_scale_factor
    this.applying=true
    try {
      if(packet.reset){this.dom.clear();this.ids=new WeakMap();this.doc.body.replaceChildren();this.doc.head.querySelectorAll('style[data-mirror-fonts],style[data-mirror-pseudo]').forEach(n=>n.remove())}
      for(const id of packet.removed){const node=this.dom.get(id);if(node?.parentNode&&node.parentNode.nodeType!==9)node.parentNode.removeChild(node);this.dom.delete(id)}
      const changed=new Set(packet.nodes.map(n=>n.id))
      const frames:Array<{record:MirrorNode;frame:HTMLIFrameElement}>=[]
      const build=(id:string,doc:Document):Node=>{
        const record=next.get(id)!;let node=this.dom.get(id)
        const old=this.records.get(id)
        const create=!node||node.ownerDocument!==doc||old?.kind!==record.kind||old?.tag!==record.tag
        if(create){const created=this.create(record,doc);node?.parentNode?.replaceChild(created,node);node=created}
        if(create||changed.has(id))this.update(record,node!,create?undefined:old)
        node=node!
        if(record.kind==='frame') {
          const frame=node as HTMLIFrameElement;frame.setAttribute('sandbox','allow-same-origin');frame.setAttribute('referrerpolicy','no-referrer');
          frames.push({record,frame})
        } else if(record.kind==='shadow'||record.kind==='document') {
          // Host attaches the fragment below; open shadow roots preserve slotting.
          // Children are reconciled directly into the host's existing root below.
          // Moving them through a fragment would discard selection and IME state.
        } else {
          const children=record.children.map(child=>build(child,doc))
          const normal=children.filter((_,i)=>next.get(record.children[i]!)?.kind!=='shadow')
          if(node.nodeType===1||node.nodeType===11) {
            const desired=new Set(normal);for(const child of Array.from(node.childNodes))if(!desired.has(child))node.removeChild(child)
            for(let i=0;i<normal.length;i++)if(node.childNodes[i]!==normal[i])node.insertBefore(normal[i]!,node.childNodes[i]??null)
            for(let i=0;i<children.length;i++)if(next.get(record.children[i]!)?.kind==='shadow') {
              const host=node as HTMLElement;const root=host.shadowRoot??host.attachShadow({mode:'open'});const shadow=next.get(record.children[i]!)!;const desired=shadow.children.map(id=>build(id,doc));for(const child of Array.from(root.childNodes))if(!desired.includes(child))root.removeChild(child);for(let j=0;j<desired.length;j++)if(root.childNodes[j]!==desired[j])root.insertBefore(desired[j]!,root.childNodes[j]??null)
            }
          }
        }
        if(record.scroll&&node.nodeType===1){(node as HTMLElement).scrollLeft=record.scroll.x;(node as HTMLElement).scrollTop=record.scroll.y}
        return node
      }
      const root=build(packet.root,this.doc) as HTMLElement
      if(root!==this.doc.documentElement){const csp=this.doc.createElement('meta');csp.httpEquiv='Content-Security-Policy';csp.content=mirrorSandboxCsp;this.doc.documentElement.replaceWith(root);this.doc.head.prepend(csp)}
      // Hydrate only after parent insertion: moving a live iframe reloads its
      // about:blank document. Nested documents inherit the inert parent CSP.
      for(let i=0;i<frames.length;i++) {
        const {record,frame}=frames[i]!;const nested=frame.contentDocument;if(!nested)throw Error('MP-11: nested mirror unavailable')
        const documentRecord=next.get(record.children[0]!)!;build(documentRecord.id,nested);const html=build(documentRecord.children[0]!,nested)
        if(html && html!==nested.documentElement){if(nested.documentElement)nested.documentElement.replaceWith(html);else nested.appendChild(html)}
        if(!nested.head.querySelector('meta[http-equiv]')){const meta=nested.createElement('meta');meta.httpEquiv='Content-Security-Policy';meta.content=mirrorSandboxCsp;nested.head.prepend(meta);this.bindEvents(nested)}
      }
      this.doc.head.querySelectorAll('style[data-mirror-fonts],style[data-mirror-pseudo]').forEach(n=>n.remove())
      const fontStyle=this.doc.createElement('style');fontStyle.dataset.mirrorFonts='true'
      for(const font of packet.fonts){const url=this.resources.get(font.resource);if(url)fontStyle.textContent+=`@font-face{font-family:${JSON.stringify(font.family.replaceAll('"',''))};src:url("${url}");font-weight:${/^[0-9 ]+$/.test(font.weight)?font.weight:'400'};font-style:${['normal','italic','oblique'].includes(font.style)?font.style:'normal'};}`}
      if(fontStyle.textContent)this.doc.head.append(fontStyle)
      const pseudoStyle=this.doc.createElement('style');pseudoStyle.dataset.mirrorPseudo='true'
      if([...next.values()].some(record=>Object.keys(record.pseudo??{}).length))this.doc.head.append(pseudoStyle)
      for(const record of next.values())for(const [pseudo,content]of Object.entries(record.pseudo??{})) {
        const node=this.dom.get(record.id) as HTMLElement;node.dataset.mirrorNode=record.id
        const probe=this.doc.createElement('span');this.style(probe,content.style)
        pseudoStyle.sheet?.insertRule(`[data-mirror-node="${record.id}"]${pseudo}{${probe.style.cssText}content:${JSON.stringify(content.text)};}`)
      }
      for(const overlay of this.overlays)overlay.remove();this.overlays=[]
      for(const {tile,url}of newTiles){const host=this.dom.get(tile.node_id) as HTMLElement;const record=next.get(tile.node_id)!;
        // Keep native controls as focus/IME targets and preserve their baseline.
        // Pixels are painted separately at integer source raster positions so a
        // fractional control box cannot resample or clip its compositor tile.
        if(record.reason==='native_control') {
          host.style.appearance='none';host.style.background='transparent';host.style.borderColor='transparent';host.style.color='transparent';host.style.setProperty('-webkit-text-fill-color','transparent');host.style.caretColor='transparent';host.removeAttribute('placeholder')
          if(host.tagName==='BUTTON'){const baseline=host.ownerDocument.createElement('span');baseline.style.visibility='hidden';baseline.textContent='M';host.replaceChildren(baseline)}
        }
        const image=host.ownerDocument.createElement('img');image.src=url
        image.style.cssText=`position:fixed;left:${record.box!.x+tile.x}px;top:${record.box!.y+tile.y}px;width:${tile.width}px;height:${tile.height}px;max-width:none;image-rendering:pixelated;pointer-events:none;z-index:2147483646;`
        host.ownerDocument.documentElement.append(image);this.overlays.push(image)
      }
      for(const record of next.values())if(record.box&&(record.kind==='mask'||['cross_origin_frame','opaque_shadow'].includes(record.reason??''))) {
        const doc=this.dom.get(record.id)!.ownerDocument!,box=record.box,scale=this.dpr
        const left=Math.floor(box.x*scale)-4,top=Math.floor(box.y*scale)-4,right=Math.ceil((box.x+box.width)*scale)+4,bottom=Math.ceil((box.y+box.height)*scale)+4
        const mask=doc.createElement('div');mask.setAttribute('aria-label','Protected content')
        mask.style.cssText=`position:fixed;left:${left/scale}px;top:${top/scale}px;width:${(right-left)/scale}px;height:${(bottom-top)/scale}px;background:black;pointer-events:none;z-index:2147483647;`
        doc.documentElement.append(mask);this.overlays.push(mask)
      }
      this.frame.contentWindow!.scrollTo(packet.scroll.x,packet.scroll.y)
      for(const url of this.tileUrls)URL.revokeObjectURL(url);this.tileUrls=newTiles.map(t=>t.url)
      const usedResources=new Set(packet.fonts.map(f=>f.resource));for(const record of next.values()){if(record.resource)usedResources.add(record.resource);for(const value of Object.values(record.style??{}))if(value.startsWith('resource:'))usedResources.add(value.slice(9))}
      for(const [id,url]of this.resources)if(!usedResources.has(id)){URL.revokeObjectURL(url);this.resources.delete(id)}
      this.records=next;this.sequence=packet.sequence;this.documentId=packet.document_id
      if(packet.selection){const selected=packet.selection,anchor=this.dom.get(selected.anchor_id)!,focus=this.dom.get(selected.focus_id)!,selection=anchor.ownerDocument!.getSelection()!;if(selection.anchorNode!==anchor||selection.anchorOffset!==selected.anchor_offset||selection.focusNode!==focus||selection.focusOffset!==selected.focus_offset)selection.setBaseAndExtent(anchor,selected.anchor_offset,focus,selected.focus_offset)}else for(const doc of new Set([...this.dom.values()].map(n=>n.ownerDocument).filter((d):d is Document=>d!==null))){const selection=doc.getSelection();if(selection&&!selection.isCollapsed)selection.removeAllRanges()}
      const focused=this.dom.get(packet.focused??'') as HTMLElement|undefined;focused?.focus?.({preventScroll:true})
    } finally {this.applying=false}
    this.timed('dom_apply',at);at=performance.now()
    await this.doc.fonts.ready
    const images=[...this.dom.values(),...this.overlays].filter((node):node is HTMLImageElement=>node.nodeType===1&&(node as Element).tagName==='IMG')
    await Promise.all(images.map(image=>image.decode().catch(()=>undefined)))
    this.timed('font_image_ready',at);this.timed('apply_total',started)
  }
  private async addResource(resource:MirrorResource):Promise<void> {
    if(await digest(bytesOf(resource.data_base64))!==resource.resource_id)throw Error('MP-11: mirror resource digest')
    if(!this.resources.has(resource.resource_id))this.resources.set(resource.resource_id,blobUrl(resource.data_base64,resource.mime_type))
  }
  // MP-10: each line fragment is measured independently, including wrapped text.
  textRunGeometry():Array<{id:string;rects:Array<{x:number;y:number;width:number;height:number}>}> {
    return [...this.records.values()].filter(record=>record.kind==='text').map(record=>{const node=this.dom.get(record.id)!;const range=node.ownerDocument!.createRange();range.selectNodeContents(node);return {id:record.id,rects:[...range.getClientRects()].map(r=>({x:r.x,y:r.y,width:r.width,height:r.height}))}})
  }
  // MP-10: diagnostics use only sanitized records and the inert client DOM.
  layoutFidelity():{boxes:number;text_nodes:number;max_error_css_px:number;text_mismatches:number;color_mismatches:number} {
    let boxes=0,text_nodes=0,max_error_css_px=0,text_mismatches=0,color_mismatches=0
    for(const [id,record]of this.records){const node=this.dom.get(id);if(!node)continue
      if(record.kind==='text'){text_nodes++;if(node.textContent!==record.text)text_mismatches++}
      if(record.box&&['element','text','frame','tile'].includes(record.kind)){let box:DOMRect;if(record.kind==='text'){const range=node.ownerDocument!.createRange();range.selectNodeContents(node);box=range.getBoundingClientRect()}else box=(node as HTMLElement).getBoundingClientRect();boxes++;for(const key of ['x','y','width','height'] as const)max_error_css_px=Math.max(max_error_css_px,Math.abs(box[key]-record.box[key]))}
      if(record.kind==='element'){const style=node.ownerDocument!.defaultView!.getComputedStyle(node as Element);for(const [key,value]of Object.entries(record.style??{}))if((key==='color'||key.endsWith('-color'))&&value!=='initial'&&value!=='inherit'&&style.getPropertyValue(key)!==value)color_mismatches++}
    }
    return {boxes,text_nodes,max_error_css_px,text_mismatches,color_mismatches}
  }
  driftNodes():string[] {
    const started=performance.now(),drift=new Set<string>()
    for(const [id,record]of this.records){if(!['element','text','frame','tile'].includes(record.kind)||!record.box)continue;const node=this.dom.get(id);if(!node)continue;let box:DOMRect;if(record.kind==='text'){const range=node.ownerDocument!.createRange();range.selectNodeContents(node);box=range.getBoundingClientRect()}else box=(node as HTMLElement).getBoundingClientRect()
      const textDrift=record.kind==='text'&&node.textContent!==record.text
      const style=['element','frame'].includes(record.kind)?node.ownerDocument!.defaultView!.getComputedStyle(node as Element):null
      const colorDrift=style&&Object.entries(record.style??{}).some(([key,value])=>(key==='color'||key.endsWith('-color'))&&value!=='initial'&&value!=='inherit'&&style.getPropertyValue(key)!==value)
      if(textDrift||colorDrift||['x','y','width','height'].some(key=>Math.abs((box[key as keyof DOMRect] as number)-record.box![key as keyof typeof record.box])>0.5)){
        let target:MirrorNode|undefined=record.kind==='element'?record:this.records.get(record.parent??'')
        while(target&&target.kind!=='element')target=this.records.get(target.parent??'')
        if(target)drift.add(target.id)
      }
      if(drift.size===64)break
    }
    this.timed('drift_scan',started);return [...drift]
  }
  private clearResources():void {for(const url of this.resources.values())URL.revokeObjectURL(url);this.resources.clear();for(const url of this.tileUrls)URL.revokeObjectURL(url);this.tileUrls=[]}
  close():void {this.disposed=true;for(const remove of this.removers)remove();this.removers=[];this.clearResources();this.dom.clear();this.records.clear();this.overlays=[];this.frame.remove()}
}
export async function attachBrowserMirror(transport:MirrorTransport,container:HTMLElement,binding:Binding,onFailure:(error:unknown)=>void):Promise<{next():Promise<MirrorPacket>;input(action:KernelBrowserMirrorAction):Promise<unknown>;takeover():Promise<unknown>;release():Promise<unknown>;actors():Promise<unknown>;close():Promise<void>;renderer:BrowserMirrorRenderer}> {
  if(!Number.isInteger(transport.protocolVersion)||transport.protocolVersion<browserMirrorMinimumProtocolVersion)throw Error('MP-08: DOM mirroring requires protocol 433')
  const request=async(command:unknown):Promise<any>=>{const response=await transport.request({KernelBrowser:{command}}) as {KernelBrowser?:{result?:unknown}};if(!response.KernelBrowser?.result)throw Error('MP-08: invalid mirror response');return response.KernelBrowser.result}
  const subscribed=await request({op:'mirror_subscribe',...binding});const subscription_id=subscribed.subscription_id as string
  let sequence=0,document_id='',closed=false,busy=false
  const input=(action:KernelBrowserMirrorAction,epoch?:{sequence:number;document_id:string}):Promise<unknown>=>{if(closed||!document_id)throw Error('MP-08: mirror has no observed document');return request({op:'mirror_input',...binding,document_id:epoch?.document_id??document_id,subscription_id,sequence:epoch?.sequence??sequence,action})}
  const renderer=new BrowserMirrorRenderer(container,input,onFailure)
  try{await renderer.ready()}catch(error){renderer.close();await request({op:'mirror_close',subscription_id,generation:binding.generation}).catch(()=>{});throw error}
  return {renderer,input,
    async next(){if(closed||busy)throw Error('MP-08: mirror credit unavailable');busy=true;try{const packet=await request({op:'mirror_next',subscription_id,generation:binding.generation,after_sequence:sequence,drift_nodes:renderer.driftNodes()}) as MirrorPacket;if(packet.subscription_id!==subscription_id||packet.tab_id!==binding.tab_id||packet.generation!==binding.generation)throw Error('MP-11: foreign mirror packet');await renderer.apply(packet);sequence=packet.sequence;document_id=packet.document_id;return packet}catch(error){closed=true;renderer.close();await request({op:'mirror_close',subscription_id,generation:binding.generation}).catch(()=>{});throw error}finally{busy=false}},
    takeover:()=>request({op:'display_takeover',tab_id:binding.tab_id,generation:binding.generation}),release:()=>request({op:'display_release',tab_id:binding.tab_id,generation:binding.generation}),actors:()=>request({op:'display_actors'}),
    async close(){if(closed)return;closed=true;renderer.close();await request({op:'mirror_close',subscription_id,generation:binding.generation})}}
}
