// MP-08/MP-11: origin resources remain inside Chromium/kernel. No fetch, cookies,
// external URL, executable CSS/SVG or profile path is sent to a mirror client.
import { createHash } from 'node:crypto';
export function mirrorCanonicalJson(value,objects=new WeakMap()) {return JSON.stringify(value,(_,v)=>{if(!v||typeof v!=='object'||Array.isArray(v))return v;let sorted=objects.get(v);if(!sorted){sorted=Object.fromEntries(Object.entries(v).sort(([a],[b])=>a<b?-1:a>b?1:0));objects.set(v,sorted)}return sorted});}
export const mirrorHash = value => createHash('sha256').update(mirrorCanonicalJson(value)).digest('hex');
// Bounds decoded memory as well as wire bytes before a client decoder sees it.
function resourceType(bytes) {
  let type=null,w=0,h=0;
  if(bytes.length>=24&&bytes.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10]))) {type='image/png';w=bytes.readUInt32BE(16);h=bytes.readUInt32BE(20);}
  else if(bytes.length>=10&&['GIF87a','GIF89a'].includes(bytes.toString('ascii',0,6))) {type='image/gif';w=bytes.readUInt16LE(6);h=bytes.readUInt16LE(8);}
  else if(bytes[0]===255&&bytes[1]===216&&bytes[2]===255) {
    for(let at=2;at+4<bytes.length;) {
      if(bytes[at++]!==255)break;const marker=bytes[at++];if(marker===217||marker===218)break;
      const length=bytes.readUInt16BE(at);if(length<2||at+length>bytes.length)break;
      if([192,193,194,195,197,198,199,201,202,203,205,206,207].includes(marker)&&length>=7){type='image/jpeg';h=bytes.readUInt16BE(at+3);w=bytes.readUInt16BE(at+5);break;}at+=length;
    }
  } else if(bytes.length>=30&&bytes.toString('ascii',0,4)==='RIFF'&&bytes.toString('ascii',8,12)==='WEBP') {
    const kind=bytes.toString('ascii',12,16);
    if(kind==='VP8X'){type='image/webp';w=bytes.readUIntLE(24,3)+1;h=bytes.readUIntLE(27,3)+1;}
    else if(kind==='VP8L'&&bytes[20]===47){type='image/webp';w=1+(bytes.readUInt32LE(21)&16383);h=1+((bytes.readUInt32LE(21)>>>14)&16383);}
    else if(kind==='VP8 '&&bytes[23]===157&&bytes[24]===1&&bytes[25]===42){type='image/webp';w=bytes.readUInt16LE(26)&16383;h=bytes.readUInt16LE(28)&16383;}
  } else if(bytes.length>=48&&['wOF2','wOFF'].includes(bytes.toString('ascii',0,4))) {
    if(bytes.readUInt32BE(8)!==bytes.length||bytes.readUInt32BE(16)>8*1024*1024||bytes.readUInt16BE(12)>256)return null;
    return bytes.toString('ascii',0,4)==='wOF2'?'font/woff2':'font/woff';
  }
  return type&&w>0&&h>0&&w<=8192&&h<=8192&&w*h<=8*1024*1024?type:null;
}
export async function materializeMirrorResources(connection,sessionId,descriptors,protectedValues,cache=new Map()) {
  if(descriptors.length>128) throw new Error('MP-11: mirror resource count');
  cache.clear();
  if(!descriptors.length)return {mapped:new Map(),resources:new Map()};
  const {frameTree}=await connection.send('Page.getResourceTree',{},sessionId),allowed=new Map();
  const collect=tree=>{for(const r of tree.resources??[]) allowed.set(r.url,tree.frame.id);for(const child of tree.childFrames??[])collect(child);};
  collect(frameTree);
  // CDP resource bodies may change at a stable URL. Cache only within this read.
  const mapped=new Map(),resources=new Map();let total=0;
  for(const item of descriptors) {
    const frameId=allowed.get(item.url);
    if(!frameId) {mapped.set(item.key,null);continue;}
    const cached=cache.get(item.url);
    if(cached){resources.set(cached.resource_id,cached);mapped.set(item.key,cached.resource_id);continue;}
    let body;
    try {body=await connection.send('Page.getResourceContent',{frameId,url:item.url},sessionId);} catch {mapped.set(item.key,null);continue;}
    if(!body.base64Encoded || typeof body.content!=='string' || body.content.length>700000) {mapped.set(item.key,null);continue;}
    const bytes=Buffer.from(body.content,'base64'),mime_type=resourceType(bytes);
    if(!mime_type || !mime_type.startsWith(item.kind==='font'?'font/':'image/')) {mapped.set(item.key,null);continue;}
    const resource_id=createHash('sha256').update(bytes).digest('hex');
    if(!resources.has(resource_id)) {
      total+=bytes.length;if(total>2*1024*1024) throw new Error('MP-11: mirror resource bytes');
      const resource={resource_id,mime_type,data_base64:bytes.toString('base64')};resources.set(resource_id,resource);cache.set(item.url,resource);
    }
    mapped.set(item.key,resource_id);
  }
  return {mapped,resources};
}

// MP-08/MP-10: canonical node strings are reusable across unchanged patches.
// Hash bytes remain identical to mirrorHash; cache entries hold sanitized data only.
export class MirrorTreeHasher {
  constructor(){this.nodes=new Map();}
  hash(source) {
    const next=new Map(),objects=new WeakMap();
    const nodes=source.nodes.map(node=>{const raw=JSON.stringify(node),old=this.nodes.get(node.id),entry=old?.raw===raw?old:{raw,canonical:mirrorCanonicalJson(node,objects)};next.set(node.id,entry);return entry.canonical;});
    this.nodes=next;
    const json=`{"focused":${mirrorCanonicalJson(source.focused)},"fonts":${mirrorCanonicalJson(source.fonts)},"nodes":[${nodes.join(',')}],"root":${mirrorCanonicalJson(source.root)},"scroll":${mirrorCanonicalJson(source.scroll)},"selection":${mirrorCanonicalJson(source.selection??null)}}`;
    return createHash('sha256').update(json).digest('hex');
  }
}
