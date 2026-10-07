// MD-454: authenticate a complete frame before creating any DOM or resource.
import {validateMirrorStyle} from './browser-mirror-security.js'
import type {MirrorPacket,MirrorNode} from './browser-mirror-types.js'
export const mirrorFrameByteLimit=32*1024*1024,mirrorChunkByteLimit=192*1024
export type MirrorChunk={kind:'mirror_chunk';subscription_id:string;tab_id:string;generation:number;document_id:string;sequence:number;index:number;parts:number;frame_bytes:number;encoding:'gzip';transfer_bytes:number;frame_sha256:string;payload_base64:string}
const sha256=async(bytes:Uint8Array):Promise<string>=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes as Uint8Array<ArrayBuffer>)),b=>b.toString(16).padStart(2,'0')).join('')
export class MirrorFrameAssembler {
 private header:MirrorChunk|null=null
 private pieces:Uint8Array[]=[]
 clear():void{this.header=null;this.pieces=[]}
 async push(chunk:MirrorChunk):Promise<MirrorPacket|null>{
  try {
   if(chunk.kind!=='mirror_chunk'||!Number.isSafeInteger(chunk.generation)||chunk.generation<0||['subscription_id','tab_id','document_id'].some(k=>typeof chunk[k as keyof MirrorChunk]!=='string'||(chunk[k as keyof MirrorChunk] as string).length<1||(chunk[k as keyof MirrorChunk] as string).length>256)||!Number.isSafeInteger(chunk.sequence)||chunk.sequence<1||!Number.isSafeInteger(chunk.frame_bytes)||chunk.frame_bytes<1||chunk.frame_bytes>mirrorFrameByteLimit||!Number.isSafeInteger(chunk.index)||chunk.index<0||!Number.isSafeInteger(chunk.parts)||chunk.encoding!=='gzip'||!Number.isSafeInteger(chunk.transfer_bytes)||chunk.transfer_bytes<1||chunk.transfer_bytes>mirrorFrameByteLimit||chunk.parts!==Math.ceil(chunk.transfer_bytes/mirrorChunkByteLimit)||chunk.index>=chunk.parts||!/^[a-f0-9]{64}$/.test(chunk.frame_sha256)||typeof chunk.payload_base64!=='string'||chunk.payload_base64.length>mirrorChunkByteLimit*4/3+4||chunk.payload_base64.length%4!==0||!/^[A-Za-z0-9+/]*={0,2}$/.test(chunk.payload_base64))throw Error('MP-11: invalid mirror chunk bounds')
   const h=this.header
   if(h&&['subscription_id','tab_id','generation','document_id','sequence','parts','frame_bytes','encoding','transfer_bytes','frame_sha256'].some(k=>chunk[k as keyof MirrorChunk]!==h[k as keyof MirrorChunk]))throw Error('MP-11: mixed mirror chunk frame')
   if(chunk.index!==this.pieces.length)throw Error('MP-11: missing or repeated mirror chunk')
   this.header??=chunk
   const bytes=Uint8Array.from(atob(chunk.payload_base64),c=>c.charCodeAt(0))
   if(bytes.length!==Math.min(mirrorChunkByteLimit,chunk.transfer_bytes-chunk.index*mirrorChunkByteLimit))throw Error('MP-11: truncated mirror chunk')
   this.pieces.push(bytes)
   if(chunk.index+1<chunk.parts)return null
   const transfer=new Uint8Array(chunk.transfer_bytes);let offset=0;for(const piece of this.pieces){transfer.set(piece,offset);offset+=piece.length}
   const reader=new Blob([transfer as Uint8Array<ArrayBuffer>]).stream().pipeThrough(new DecompressionStream('gzip')).getReader()
   const frame=new Uint8Array(chunk.frame_bytes);let at=0
   try{while(true){const {done,value}=await reader.read();if(done)break;if(value.length>frame.length-at)throw Error('MP-11: mirror decompression exceeds memory budget');frame.set(value,at);at+=value.length}}catch(error){await reader.cancel().catch(()=>{});throw error}finally{reader.releaseLock()}
   if(at!==frame.length)throw Error('MP-11: truncated mirror frame')
   if(await sha256(frame)!==chunk.frame_sha256)throw Error('MP-11: mirror frame digest mismatch')
   const wire=JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(frame)) as Omit<MirrorPacket,'nodes'>&{styles:Record<string,string>[];nodes:Array<MirrorNode&{style_ref?:number}>}
   if(['subscription_id','tab_id','generation','document_id','sequence'].some(k=>wire[k as keyof MirrorPacket]!==chunk[k as keyof MirrorChunk]))throw Error('MP-11: foreign mirror frame')
   if(!Array.isArray(wire.styles)||wire.styles.length>100000||!Array.isArray(wire.nodes)||wire.nodes.length>100000)throw Error('MP-11: mirror working set bounds')
   for(const style of wire.styles){if(!style||typeof style!=='object'||Array.isArray(style))throw Error('MP-11: invalid mirror CSS map');validateMirrorStyle(style)}
   const nodes=wire.nodes.map(node=>{if(node.style_ref===undefined)return node;if(node.style||!Number.isSafeInteger(node.style_ref)||node.style_ref<0||!wire.styles[node.style_ref])throw Error('MP-11: invalid mirror CSS reference');const {style_ref,...record}=node;return {...record,style:wire.styles[style_ref]!}})
   const {styles,...packet}=wire
   this.clear();return {...packet,nodes}
  }catch(error){this.clear();throw error}
 }
}

// Observation-only: no partial frame may reach the renderer or input epoch.
export async function readMirrorFrame(read:(cursor:number|null,restarted:boolean)=>Promise<MirrorPacket|MirrorChunk>,subscription:string,tab:string,generation:number):Promise<MirrorPacket>{
 const assembler=new MirrorFrameAssembler();let cursor:number|null=null,restarted=false
 while(true){
  let value:MirrorPacket|MirrorChunk
  try{value=await read(cursor,restarted)}catch(error){
   assembler.clear()
   if(!restarted&&error instanceof Error&&/\bMP-11: mirror frame changed before commit\b/.test(error.message)){
    restarted=true;cursor=null;await new Promise(resolve=>setTimeout(resolve,50));continue
   }
   throw error
  }
  if(value.subscription_id!==subscription||value.tab_id!==tab||value.generation!==generation)throw Error('MP-11: foreign mirror packet')
  if('kind'in value&&value.kind==='mirror_chunk'){
   const packet=await assembler.push(value);if(packet)return packet;cursor=value.index;continue
  }
  if(cursor!==null)throw Error('MP-11: mirror chunk interrupted by unrelated packet')
  return value as MirrorPacket
 }
}
