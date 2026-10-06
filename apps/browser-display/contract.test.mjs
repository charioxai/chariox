// MD-DISPLAY-04: pin actual adapter packet metadata together with protocol 443.
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
test('MD-DISPLAY actual video/PNG/tile packet contract at protocol 443', async()=>{
 const version=await readFile(new URL('../../packages/kernel-client/src/kernel-types.ts',import.meta.url),'utf8');
 assert.match(version,/LOCAL_DAEMON_PROTOCOL_VERSION = 443\b/);
 const frames=await contract();
 assert.equal(createHash('sha256').update(JSON.stringify(frames)).digest('hex'),'2ee8a6385402e07acc6ea78752e7f6832ea269faa9646362475c6f5705c672a6');
});

test('MD-DISPLAY443 pins explicit dependency admission and key/delta/IDR contract',async()=>{
 const {minimumProtocolVersion,supportedCodecs}=await import('./presenter.mjs');
 assert.equal(minimumProtocolVersion,443);
 assert.ok((await supportedCodecs()).includes('chariox-video-dependencies-v1'));
 const binding={subscription_id:'s',tab_id:'t',device_scale_factor:1,bitrate:2000000,codec:'vp09.00.10.08',dependencies:true};
 const resets=[];const stream=new DisplayStream(binding,{encoder:{encode:async(p,b,key)=>{resets.push(key);return{key,data_base64:'opaque'}},close:async()=>{}},now:()=>0,wait:async()=>{}});
 try{for(let n=0;n<3;n++)await stream.frame({generation:1,motion:true,width:1280,height:800,data_base64:String(n)},'document',stream.sequence);assert.deepEqual(resets,[true,false,false]);stream.invalidate();await stream.frame({generation:1,motion:true,width:1280,height:800,data_base64:'recover'},'document',stream.sequence);assert.equal(resets.at(-1),true)}finally{await stream.close()}
});
