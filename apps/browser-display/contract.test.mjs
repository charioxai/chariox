// MD-DISPLAY-04: pin actual adapter packet metadata together with protocol 466.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { DisplayStream } from '../kernel/slice-linux-docker/docker/kernel-browser-display.mjs';
import { encodePng } from '../kernel/slice-linux-docker/docker/kernel-browser-pixels.mjs';
export async function contract() {
 const binding={subscription_id:'s',tab_id:'t',device_scale_factor:2,bitrate:2_000_000,codec:'vp09.00.10.08'};
 const source={generation:2,data_base64:encodePng(256,256,Buffer.alloc(256*256*4,255))};
 const stream=new DisplayStream(binding,{encoder:{encode:async()=> 'a2V5',close:async()=>{}},now:()=>0,wait:async()=>{}});
 const first=await stream.frame(source,'d',0),settled=await stream.frame(source,'d',1);
 const changed=Buffer.alloc(256*256*4,255);changed[0]=0;
 const patch=await stream.frame({generation:2,data_base64:encodePng(256,256,changed)},'d',2);
 const sanitize=value=>JSON.parse(JSON.stringify(value,(key,value)=>key==='data_base64'?'<opaque>':value));
 return [first,settled,patch].map(sanitize);
}
test('MD-DISPLAY actual video/PNG/tile packet contract at protocol 466', async()=>{
 const version=await readFile(new URL('../../packages/kernel-client/src/kernel-types.ts',import.meta.url),'utf8');
 assert.match(version,/LOCAL_DAEMON_PROTOCOL_VERSION = 491\b/);
 const frames=await contract();
 assert.equal(createHash('sha256').update(JSON.stringify(frames)).digest('hex'),'ad69928619a971cc480d4eb2767432a3cbcb8d320469c0de5eca1c39bd1493d3');
});

test('MD-DISPLAY443 pins explicit dependency admission and key/delta/IDR contract',async()=>{
 const {minimumProtocolVersion,pushProtocolVersion,supportedCodecs}=await import('./presenter.mjs');
 assert.equal(minimumProtocolVersion,466);assert.equal(pushProtocolVersion,491);
 assert.ok((await supportedCodecs()).includes('chariox-video-dependencies-v1'));
 const binding={subscription_id:'s',tab_id:'t',device_scale_factor:1,bitrate:2000000,codec:'vp09.00.10.08',dependencies:true};
 const resets=[];const stream=new DisplayStream(binding,{encoder:{encode:async(p,b,key)=>{resets.push(key);return{key,data_base64:'opaque'}},close:async()=>{}},now:()=>0,wait:async()=>{}});
 try{for(let n=0;n<3;n++)await stream.frame({generation:1,motion:true,width:1280,height:800,data_base64:String(n)},'document',stream.sequence);assert.deepEqual(resets,[true,false,false]);stream.invalidate();await stream.frame({generation:1,motion:true,width:1280,height:800,data_base64:'recover'},'document',stream.sequence);assert.equal(resets.at(-1),true)}finally{await stream.close()}
});

test('MP-08/MP-10/MP-11 protocol466/peer96 pins stripe offer and atomic row packet',async()=>{
 const local=await readFile(new URL('../../packages/kernel-client/src/kernel-types.ts',import.meta.url),'utf8');
 const peer=await readFile(new URL('../kernel/src/transport/relay_peer.rs',import.meta.url),'utf8');
 assert.match(local,/LOCAL_DAEMON_PROTOCOL_VERSION = 491\b/);assert.match(peer,/RELAY_PEER_PROTOCOL_VERSION: u32 = 101\b/);
 const stream=new DisplayStream({subscription_id:'s',tab_id:'t',device_scale_factor:1,bitrate:8000000,codec:'avc1.420033',dependencies:true,stripes:true,css_width:1920,css_height:1080},{encoder:{close:async()=>{}},now:()=>0,wait:async()=>{}});
 const stripes=Array.from({length:8},(_,row)=>({row,y:2*Math.floor(540*row/8),height:2*(Math.floor(540*(row+1)/8)-Math.floor(540*row/8)),codec:'avc1.420033',key:true,sequence:1,reference_sequence:null,data_base64:'opaque'}));
 try{
  const packet=await stream.frame({generation:2,motion:true,width:1920,height:1080,data_base64:'native',encoded:{stripes}},'d',0);
  assert.equal(packet.kind,'stripes');assert.equal(packet.base_sequence,0);assert.equal(packet.stripes.length,8);
  assert.equal(createHash('sha256').update(JSON.stringify(packet)).digest('hex'),'3bae9dfbec8577eabf9e740271d72d8988f3c933e44d1f02946eb0af9607396a');
 }finally{await stream.close()}
});
