// MP-08/MP-10/MP-11: actual helper output, discard and handoff ownership.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,readFile,rm,readdir,unlink} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {pathToFileURL} from 'node:url';
const {PortableEncoder,DisplayStream}=await import(process.env.MP_PACKET_MODULE
  ? pathToFileURL(process.env.MP_PACKET_MODULE) : './kernel-browser-display.mjs');

test('MP-10 real native row bytes bypass Node and discard only unsent packets',async()=>{
 const root=await mkdtemp(tmpdir()+'/chariox-mp10-packets-');
 const before=process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT;process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=root;
 const encoder=new PortableEncoder(),raw={width:128,height:128,format:'bgr0',length:128*128*4,pixels:Buffer.alloc(128*128*4)};
 try{
  const encoded=await encoder.encodeStripes(raw,8000000,true);
  assert.ok(encoded.packet,'native bytes must be in a private packet, not the Node response');
  assert.equal(encoded.stripes.length,8);assert.ok(encoded.stripes.every(r=>!Object.hasOwn(r,'data_base64')));
  const bytes=await readFile(root+'/'+encoded.packet.name);assert.equal(bytes.length,encoded.packet.length);
  // MP-08/MP-10: protocol 466 packet = u32 header length, JSON headers, raw rows.
  const size=bytes.readUInt32BE(0),headers=JSON.parse(bytes.subarray(4,4+size));
  assert.deepEqual(headers,encoded.stripes);assert.ok(headers.every(r=>Number.isSafeInteger(r.length)&&r.length>0));
  assert.equal(4+size+headers.reduce((n,r)=>n+r.length,0),bytes.length,'raw segments cover the packet exactly');
  assert.equal(bytes.readUInt32BE(4+size),1,'raw AnnexB row bytes, not base64 text');
  encoder.discard(encoded);
  const remaining=await readdir(root);assert.equal(remaining.length,1);assert.match(remaining[0],/^encoder-/);
  assert.deepEqual(await readdir(root+'/'+remaining[0]),['raster'],'only the bounded active request snapshot remains');
  const second=await encoder.encodeStripes(raw,8000000,true);encoder.handedOff(second);
  await encoder.close();assert.deepEqual(await readdir(root),[second.packet.name],'only Rust consumes a handed-off packet');
  await unlink(root+'/'+second.packet.name);
 }finally{await encoder.close();await rm(root,{recursive:true,force:true});if(before===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT;else process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=before}
});

test('MP-11 failed or oversized display packets retire native bytes',async()=>{
 const discarded=[],encoder={close:async()=>{},discard:e=>discarded.push(e),handedOff:()=>assert.fail('must not hand off refused pixels')};
 const stream=new DisplayStream({subscription_id:'s',tab_id:'t',device_scale_factor:1,bitrate:8000000,codec:'avc1.420033',dependencies:true},{encoder,now:()=>0,wait:async()=>{}});
 const encoded={stripes:Array.from({length:8},(_,row)=>({row,key:true})),packet:{name:'a'.repeat(32)+'.json',length:700000}};
 try{
  await assert.rejects(stream.frame({motion:true,generation:1,width:1280,height:800,data_base64:'a',encoded:{...encoded,packet:{...encoded.packet,length:1024*1024}}},'d',0),/bounded egress/);
  assert.equal(discarded.length,1);
  assert.equal(await stream.frame({motion:true,generation:1,width:1280,height:800,data_base64:'b',encoded},'d',0,async()=>false),null);
  assert.equal(discarded.length,2);
 }finally{await stream.close()}
});
