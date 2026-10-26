// MP-08/MP-10/MP-11: actual Rust-prepared repair through private IPC and packet batching.
// Supplementary regression only; the live hosted viewer proves final pixels.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {NativePipe} from '../kernel/slice-linux-docker/docker/kernel-browser-native-pipe.mjs';
import {NativeWorkerControl} from '../kernel/slice-linux-docker/docker/kernel-browser-native-worker.mjs';
import {DisplayStream} from '../kernel/slice-linux-docker/docker/kernel-browser-display.mjs';
for(const relay_binary of [false,true])test(`MP-08/MP-10/MP-11 actual Retina repair fits ${relay_binary?'binary':'legacy'} egress`,{skip:!process.env.CHARIOX_RETINA_REPLY_FIXTURE},async()=>{
 const wire=await readFile(process.env.CHARIOX_RETINA_REPLY_FIXTURE);
 const control=new NativeWorkerControl({stdin:{destroyed:false,write(_bytes,done){done?.()}}});
 control.sequence=6;
 const pending=control.request('exact',{serial:1});
 const pipe=new NativePipe(h=>control.validate(h),(h,b)=>control.receive(h,b));
 for(let at=0;at<wire.length;at+=16381)pipe.push(wire.subarray(at,at+16381));
 const exact=await pending;control.close();
 const stream=new DisplayStream({subscription_id:'retina',tab_id:'fixture',generation:1,device_scale_factor:2,bitrate:8_000_000,codec:'avc1.420033',relay_binary},{encoder:{close:async()=>{}},now:()=>0,wait:async()=>{}});
 // Start from an admitted motion base; this test covers exact packet delivery.
 stream.sequence=1;stream.document_id='d';stream.previous={width:2560,height:1600,signature:'motion',pixels:null};stream.exact=false;
 let clips=0,area=0;
 try{
  const source={...exact,data_base64:'0000000000000007',generation:1,settled_verified:true,refinement_serial:7,push_delivery:true};
  for(let credit=0;credit<100&&!stream.exact;credit++){
   const packet=await stream.frame(source,'d',stream.sequence);
   assert.equal(packet.kind,'tiles');
   assert.ok(Buffer.byteLength(JSON.stringify(packet))+1024<1024*1024,'each legacy envelope remains below1MiB');
   clips+=packet.tiles.length;area+=packet.tiles.reduce((sum,t)=>sum+t.width*t.height,0);
   assert.equal(stream.exact,clips===exact.repair_tiles.length,'full fidelity certifies only the final clip');
  }
  assert.equal(stream.exact,true);assert.equal(clips,exact.repair_tiles.length);assert.equal(area,2560*1600);
 }finally{await stream.close();}
});
