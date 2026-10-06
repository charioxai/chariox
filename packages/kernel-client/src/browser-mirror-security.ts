// MP-11: independent client validation precedes DOM construction or Blob creation.
import type { MirrorNode, MirrorPacket } from './browser-mirror-types.js'
export const mirrorTags = new Set('html head body div span p a article section main header footer nav aside h1 h2 h3 h4 h5 h6 ul ol li dl dt dd pre code blockquote b strong i em u s small sub sup br hr table thead tbody tfoot tr th td caption colgroup col input textarea button select option optgroup label fieldset legend form details summary dialog img figure figcaption picture source slot iframe'.split(' '))
const attributes = new Set('title alt role aria-label aria-hidden aria-expanded aria-checked aria-selected aria-disabled slot dir lang colspan rowspan span type placeholder disabled readonly multiple size rows cols wrap open start reversed value checked selected contenteditable'.split(' '))
const forbiddenCss = /url\s*\(|image-set\s*\(|(?:-webkit-)?image\s*\(|expression\s*\(|@|\\|[<>]|[\u0000-\u0008]/i
export const mirrorSandboxCsp = "default-src 'none'; script-src 'none'; connect-src 'none'; img-src blob:; font-src blob:; style-src 'unsafe-inline'; frame-src 'self' about:; base-uri 'none'; form-action 'none'; object-src 'none'"
export function validateMirrorStyle(style: Record<string,string>): void {
  for(const [property,value] of Object.entries(style)) {
    if(!/^-?[a-z][a-z-]*$/.test(property) || ['content','cursor'].includes(property) || typeof value!=='string' || value.length>2048 || !/^resource:[a-f0-9]{64}$/.test(value) && forbiddenCss.test(value)) throw Error('MP-11: unsafe mirror CSS')
  }
}
export function validateMirrorNode(node: MirrorNode): void {
  if(!/^n[1-9][0-9]*$/.test(node.id) || !Array.isArray(node.children) || node.children.length>12000 || !['element','text','shadow','document','frame','mask','tile'].includes(node.kind)) throw Error('MP-11: invalid mirror node')
  if(node.tag && !mirrorTags.has(node.tag)) throw Error('MP-11: active mirror element')
  if(node.attributes) for(const [key,value] of Object.entries(node.attributes)) if(!(attributes.has(key)||node.tag==='slot'&&key==='name') || typeof value!=='string' || value.length>2048) throw Error('MP-11: unsafe mirror attribute')
  if(node.attributes?.contenteditable!==undefined && !['true','false','plaintext-only'].includes(node.attributes.contenteditable))throw Error('MP-11: unsafe mirror editable state')
  if(node.attributes?.type && !['text','search','email','url','number','tel','checkbox','radio','range','button','submit','reset','date','time','color','hidden'].includes(node.attributes.type)) throw Error('MP-11: protected/active form type')
  if(node.text!==undefined && (typeof node.text!=='string' || node.text.length>2097152)) throw Error('MP-11: invalid mirror text')
  if(node.style) validateMirrorStyle(node.style)
  if(node.box && Object.values(node.box).some(n=>!Number.isFinite(n) || Math.abs(n)>10000000)) throw Error('MP-11: invalid mirror geometry')
  if(node.resource && !/^[a-f0-9]{64}$/.test(node.resource)) throw Error('MP-11: invalid mirror resource')
  for(const [pseudo,content] of Object.entries(node.pseudo??{})) {if(!['::before','::after'].includes(pseudo) || typeof content.text!=='string') throw Error('MP-11: unsafe mirror pseudo');validateMirrorStyle(content.style)}
  if(node.kind==='mask' && (node.text || node.resource || node.form || node.children.length || node.pseudo)) throw Error('MP-11: protected mirror payload')
}
export function validateMirrorPacket(packet: MirrorPacket, previous: ReadonlyMap<string,MirrorNode>): Map<string,MirrorNode> {
  if(JSON.stringify(packet).length>4194304 || !Number.isSafeInteger(packet.sequence) || packet.sequence<=0 || !/^[a-f0-9]{64}$/.test(packet.hash) || packet.css_width!==1280 || packet.css_height!==800 || ![1,2].includes(packet.device_scale_factor) || packet.nodes.length>12000 || packet.resources.length>128 || packet.tiles.length>64) throw Error('MP-11: mirror packet bounds')
  const next = packet.reset ? new Map<string,MirrorNode>() : new Map(previous)
  for(const id of packet.removed) next.delete(id)
  const changed = new Set<string>(),styles=new Map<string,Record<string,string>>()
  for(const node of packet.nodes) {validateMirrorNode(node);if(changed.has(node.id))throw Error('MP-11: duplicate mirror node');changed.add(node.id);const copy=structuredClone(node);if(copy.style){const key=JSON.stringify(copy.style);if(!styles.has(key))styles.set(key,Object.freeze(copy.style));copy.style=styles.get(key)!}next.set(node.id,copy)}
  if(next.size>12000) throw Error('MP-11: mirror tree bounds')
  const seen=new Set<string>()
  const visit=(id:string,parent:string|null,depth:number):void=>{
    const node=next.get(id)
    if(!node || node.parent!==parent || seen.has(id) || depth>128)throw Error('MP-11: invalid mirror tree')
    seen.add(id);for(const child of node.children)visit(child,id,depth+1)
  }
  visit(packet.root,null,0);if(seen.size!==next.size)throw Error('MP-11: unreachable mirror nodes')
  if(packet.selection){const s=packet.selection,a=next.get(s.anchor_id),b=next.get(s.focus_id);if(a?.kind!=='text'||b?.kind!=='text'||!Number.isInteger(s.anchor_offset)||!Number.isInteger(s.focus_offset)||s.anchor_offset<0||s.focus_offset<0||s.anchor_offset>(a.text?.length??0)||s.focus_offset>(b.text?.length??0))throw Error('MP-11: unsafe mirror selection')}
  for(const resource of packet.resources) if(!/^[a-f0-9]{64}$/.test(resource.resource_id)||!['image/png','image/jpeg','image/gif','image/webp','font/woff','font/woff2'].includes(resource.mime_type)||typeof resource.data_base64!=='string'||resource.data_base64.length>700000) throw Error('MP-11: executable/oversized mirror resource')
  for(const tile of packet.tiles) if(next.get(tile.node_id)?.kind!=='tile'||[tile.x,tile.y,tile.width,tile.height].some(n=>!Number.isFinite(n))||tile.width<=0||tile.height<=0||tile.data_base64.length>4194304)throw Error('MP-11: invalid mirror tile')
  return next
}

export function mirrorCanonicalJson(value: unknown,objects=new WeakMap<object,unknown>()): string {return JSON.stringify(value,(_,v:unknown)=>{if(!v||typeof v!=='object'||Array.isArray(v))return v;let sorted=objects.get(v);if(!sorted){sorted=Object.fromEntries(Object.entries(v).sort(([a],[b])=>a<b?-1:a>b?1:0));objects.set(v,sorted)}return sorted})}

// MP-08/MP-10: the immutable packet node object is the cache identity.
export function mirrorTreeCanonicalJson(source:{root:string;nodes:MirrorNode[];fonts:unknown;scroll:unknown;focused:unknown;selection:unknown},cache:WeakMap<MirrorNode,string>):string {
  const objects=new WeakMap<object,unknown>()
  const nodes=source.nodes.map(node=>{let serialized=cache.get(node);if(!serialized){serialized=mirrorCanonicalJson(node,objects);cache.set(node,serialized)}return serialized})
  return `{"focused":${mirrorCanonicalJson(source.focused)},"fonts":${mirrorCanonicalJson(source.fonts)},"nodes":[${nodes.join(',')}],"root":${mirrorCanonicalJson(source.root)},"scroll":${mirrorCanonicalJson(source.scroll)},"selection":${mirrorCanonicalJson(source.selection)}}`
}
