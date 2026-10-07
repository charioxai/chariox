// MD-454: bounded, retry-stable credits. Partial frames never authorize input.
import {gzipSync} from 'node:zlib';
import {createHash} from 'node:crypto';
export const mirrorWireProtocolVersion=454,mirrorWirePeerVersion=94;
export const mirrorFrameBytes=32*1024*1024,mirrorChunkBytes=192*1024;
export function mirrorChunks(packet) {
  if(Buffer.byteLength(JSON.stringify(packet))<=mirrorChunkBytes)return null;
  const styles=[],ids=new Map();
  const nodes=packet.nodes.map(node=>{if(!node.style)return node;const key=JSON.stringify(node.style);let style_ref=ids.get(key);if(style_ref===undefined){style_ref=styles.length;ids.set(key,style_ref);styles.push(node.style);}const {style,...record}=node;return {...record,style_ref};});
  const bytes=Buffer.from(JSON.stringify({...packet,nodes,styles}));
  if(bytes.length>mirrorFrameBytes)throw Error('MP-11: mirror working set exceeds memory budget');
  const frame_sha256=createHash('sha256').update(bytes).digest('hex'),transfer=gzipSync(bytes,{level:1}),parts=Math.ceil(transfer.length/mirrorChunkBytes);
  return Array.from({length:parts},(_,index)=>({kind:'mirror_chunk',subscription_id:packet.subscription_id,tab_id:packet.tab_id,generation:packet.generation,document_id:packet.document_id,sequence:packet.sequence,index,parts,frame_bytes:bytes.length,encoding:'gzip',transfer_bytes:transfer.length,frame_sha256,payload_base64:transfer.subarray(index*mirrorChunkBytes,(index+1)*mirrorChunkBytes).toString('base64')}));
}
