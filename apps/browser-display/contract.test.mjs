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
test('MD-DISPLAY actual video/PNG/tile packet contract at protocol 469', async()=>{
 const version=await readFile(new URL('../../packages/kernel-client/src/kernel-types.ts',import.meta.url),'utf8');
 assert.match(version,/LOCAL_DAEMON_PROTOCOL_VERSION = 481\b/);
 const frames=await contract();
 assert.equal(createHash('sha256').update(JSON.stringify(frames)).digest('hex'),'2ee8a6385402e07acc6ea78752e7f6832ea269faa9646362475c6f5705c672a6');
});
