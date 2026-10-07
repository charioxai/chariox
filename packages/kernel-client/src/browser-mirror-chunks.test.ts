import test from 'node:test'
import assert from 'node:assert/strict'
import {createHash,randomBytes} from 'node:crypto'
import {gzipSync} from 'node:zlib'
import {MirrorFrameAssembler,readMirrorFrame,mirrorChunkByteLimit,type MirrorChunk} from './browser-mirror-chunks.js'
const wire=(style:Record<string,string>={color:'black'})=>({subscription_id:'s',tab_id:'t',generation:1,document_id:'d',sequence:1,base_sequence:null,reset:true,hash:'a'.repeat(64),root:'n1',nodes:[{id:'n1',parent:null,children:[],kind:'element',tag:'p',style_ref:0,attributes:{title:randomBytes(400000).toString('base64')}}],styles:[style],removed:[],resources:[],tiles:[],fonts:[],scroll:{x:0,y:0},focused:null,selection:null,css_width:1280,css_height:800,device_scale_factor:1})
function chunks(value=wire()):MirrorChunk[]{const frame=Buffer.from(JSON.stringify(value)),transfer=gzipSync(frame),parts=Math.ceil(transfer.length/mirrorChunkByteLimit);return Array.from({length:parts},(_,index)=>({kind:'mirror_chunk',subscription_id:'s',tab_id:'t',generation:1,document_id:'d',sequence:1,index,parts,frame_bytes:frame.length,encoding:'gzip',transfer_bytes:transfer.length,frame_sha256:createHash('sha256').update(frame).digest('hex'),payload_base64:transfer.subarray(index*mirrorChunkByteLimit,(index+1)*mirrorChunkByteLimit).toString('base64')}))}
test('MD-454: no complete frame or CSS escapes until all bounded chunks authenticate',async()=>{const assembler=new MirrorFrameAssembler(),parts=chunks();assert(parts.length>1);for(const part of parts.slice(0,-1))assert.equal(await assembler.push(part),null);const packet=await assembler.push(parts.at(-1)!);assert.equal(packet!.nodes[0]!.style!.color,'black');assert(!('style_ref'in packet!.nodes[0]!));assert(!('styles'in packet!))})
test('MD-454: missing, repeated, foreign or modified chunks discard the entire partial frame',async()=>{for(const mode of ['skip','repeat','foreign','modified']){const assembler=new MirrorFrameAssembler(),parts=chunks();await assembler.push(parts[0]!);const bad={...parts[mode==='skip'?2:1]!};if(mode==='repeat')bad.index=0;if(mode==='foreign')bad.document_id='other';if(mode==='modified')bad.payload_base64='AA==';await assert.rejects(assembler.push(bad));await assert.rejects(assembler.push(parts[1]!))}})
test('MD-454: invalid CSS and semantic substitutions never become a rendered frame',async()=>{for(const bad of [wire({background:'url(https://origin.invalid/secret)'}),{...wire(),tab_id:'foreign'}]){const assembler=new MirrorFrameAssembler();let refused=false;for(const part of chunks(bad)){try{await assembler.push(part)}catch{refused=true;break}}assert(refused)}})
test('MD-454: decompression budget is enforced before accepting a compressed bomb',async()=>{const parts=chunks({...wire(),nodes:[]});const assembler=new MirrorFrameAssembler();let refused=false;for(const part of parts){try{await assembler.push({...part,frame_bytes:100})}catch{refused=true;break}}assert(refused)})
test('MD-454: a changed decoded frame digest is refused',async()=>{const assembler=new MirrorFrameAssembler();let refused=false;for(const part of chunks()){try{await assembler.push({...part,frame_sha256:'b'.repeat(64)})}catch{refused=true;break}}assert(refused)})

test('MD-454: an uncommitted source race discards partial bytes and restarts once',async()=>{
 const parts=chunks(),seen:Array<number|null>=[];let call=0;
 const packet=await readMirrorFrame(async cursor=>{seen.push(cursor);call++;if(call===1)return parts[0]!;if(call===2)throw Error('local transport failed: MP-11: mirror frame changed before commit');return parts[cursor===null?0:cursor+1]!},'s','t',1);
 assert.equal(packet.sequence,1);assert.deepEqual(seen.slice(0,3),[null,0,null]);
});
test('MD-454: repeated frame races and unrelated protection errors never render',async()=>{
 for(const message of ['MP-11: mirror frame changed before commit','MP-11: stale mirror protection policy']){
  let calls=0;await assert.rejects(readMirrorFrame(async()=>{calls++;throw Error(message)},'s','t',1));assert.equal(calls,message.includes('frame changed')?2:1);
 }
});
