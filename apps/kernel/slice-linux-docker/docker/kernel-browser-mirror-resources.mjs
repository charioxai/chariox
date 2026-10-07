// MP-08/MP-11: origin resources remain inside Chromium/kernel. No fetch, cookies,
// external URL, executable CSS/SVG or profile path is sent to a mirror client.
import { createHash } from 'node:crypto';
import { crc32 } from 'node:zlib';
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
    return {mime_type:bytes.toString('ascii',0,4)==='wOF2'?'font/woff2':'font/woff',decoded_bytes:bytes.readUInt32BE(16)};
  }
  return type&&w>0&&h>0&&w<=8192&&h<=8192&&w*h<=8*1024*1024?{mime_type:type,decoded_bytes:w*h*4}:null;
}
// Large static PNGs travel in bounded 454 chunks. Check the complete encoded
// container before admitting the larger budget; animated containers stay refused.
function staticPng(bytes) {
  let at=8,header=false,data=false,end=false;
  while(at+12<=bytes.length) {
    const size=bytes.readUInt32BE(at),type=bytes.toString('ascii',at+4,at+8),next=at+size+12;
    if(next>bytes.length||crc32(bytes.subarray(at+4,next-4))!==bytes.readUInt32BE(next-4)||['acTL','fcTL','fdAT'].includes(type))return false;
    if(!header){if(type!=='IHDR'||size!==13||bytes[at+16]>8)return false;header=true;}
    else if(type==='IHDR')return false;
    if(type==='IDAT')data=true;
    if(type==='IEND'){if(size!==0||!data||next!==bytes.length)return false;end=true;break;}
    at=next;
  }
  return header&&data&&end;
}
export async function materializeMirrorResources(connection,sessionId,descriptors,protectedValues,cache=new Map(),{loadedFontBody}={}) {
  if(protectedValues.length && descriptors.length) throw new Error('MP-11: protected resources refused');
  if(!Array.isArray(descriptors)||descriptors.length>100000)throw Error('MP-11: mirror descriptor working set exceeds memory budget');
  const readUrls=new Map();
  if(!descriptors.length){cache.clear();return {mapped:new Map(),resources:new Map(),encodedBytes:0,decodedBytes:0};}
  const {frameTree}=await connection.send('Page.getResourceTree',{},sessionId),allowed=new Map();
  const collect=tree=>{for(const r of tree.resources??[]) allowed.set(r.url,tree.frame.id);for(const child of tree.childFrames??[])collect(child);};
  collect(frameTree);
  // CDP resource bodies may change at a stable URL. Cache only within this read.
  const mapped=new Map(),resources=new Map();let total=0,decodedTotal=0;
  for(const item of descriptors) {
    const frameId=allowed.get(item.url);
    if(!frameId) {mapped.set(item.key,null);continue;}
    const urlKey=JSON.stringify([item.url,item.kind]),cached=readUrls.get(urlKey);
    if(readUrls.has(urlKey)){if(cached)resources.set(cached.resource_id,cached);mapped.set(item.key,cached?.resource_id??null);continue;}
    readUrls.set(urlKey,null);
    let body;
    try {body=await connection.send('Page.getResourceContent',{frameId,url:item.url},sessionId);}catch{
      if(item.kind==='font'&&loadedFontBody)try{body=await loadedFontBody(item.url)}catch{}
      if(!body){mapped.set(item.key,null);continue;}
    }
    if(!body.base64Encoded || typeof body.content!=='string' || body.content.length>4194304) {mapped.set(item.key,null);continue;}
    const bytes=Buffer.from(body.content,'base64'),metadata=resourceType(bytes);
    if(!metadata || !metadata.mime_type.startsWith(item.kind==='font'?'font/':'image/')) {mapped.set(item.key,null);continue;}
    if(body.content.length>700000 && (!['image/png','image/jpeg'].includes(metadata.mime_type) || metadata.mime_type==='image/png'&&!staticPng(bytes))) {mapped.set(item.key,null);continue;}
    const resource_id=createHash('sha256').update(bytes).digest('hex');
    if(!resources.has(resource_id)) {
      // Rasterize only the unavailable region if its decoder working set would
      // exceed the budget; never reject the entire DOM due to resource count.
      if(total+bytes.length>16*1024*1024||decodedTotal+metadata.decoded_bytes>64*1024*1024){mapped.set(item.key,null);continue;}
      total+=bytes.length;decodedTotal+=metadata.decoded_bytes;
      const resource=cache.get(resource_id)??{resource_id,mime_type:metadata.mime_type,data_base64:bytes.toString('base64')};resources.set(resource_id,resource);cache.set(resource_id,resource);
    }
    readUrls.set(JSON.stringify([item.url,item.kind]),resources.get(resource_id));
    mapped.set(item.key,resource_id);
  }
  for(const resourceId of cache.keys())if(!resources.has(resourceId))cache.delete(resourceId);
  return {mapped,resources,encodedBytes:total,decodedBytes:decodedTotal};
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
