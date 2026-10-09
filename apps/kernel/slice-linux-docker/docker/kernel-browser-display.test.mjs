import test from 'node:test';
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { DisplayStream, PortableEncoder, safeChildPid, dirtyTiles } from './kernel-browser-display.mjs';
import { encodePng, decodePng, maskPng, displayMaskRegions } from './kernel-browser-pixels.mjs';
import { BrowserEncoder } from './kernel-browser-webcodecs.mjs';
function fixture(value = 255) {
  const pixels = Buffer.alloc(256 * 256 * 4, 255); pixels[0] = value;
  return { generation: 1, data_base64: encodePng(256, 256, pixels) };
}
const binding = { subscription_id: 's', tab_id: 't', device_scale_factor: 2, bitrate: 2_000_000, codec: 'vp09.00.10.08', dependencies:true };
test('MP-08/MP-10/MP-11 contiguous small input patches a lossy canvas without certifying the whole viewport',async()=>{
 const stream=new DisplayStream(binding,{now:()=>0,wait:async()=>{},encoder:{close:async()=>{}}});
 const raw={nativeExact:async()=>{},format:'bgr0',width:1280,height:800,length:1280*800*4,pixels:Buffer.alloc(1280*800*4),damage:[0,0,1280,800],base_serial:1,serial:11,adjacent_damage_tiles:[[0,0,32,32]]};
 stream.sequence=7;stream.document_id='d';stream.previous={signature:'lossy'};stream.compositorSerial=10;stream.compositorMasks='[]';
 try{
  assert.equal(stream.canPatchNative({serial:11,raw}),true,'a delivered contiguous lossy source supports opaque small damage');
  assert.equal(stream.canPatchNative({serial:12,raw:{...raw,serial:12}}),false,'dropped capture cannot authorize adjacent reuse');
  assert.equal(stream.canPatchNative({serial:11,raw:{...raw,adjacent_damage_tiles:[[0,0,1280,800]]}}),false,'dense motion still uses video');
  stream.compositorMasks='changed';assert.equal(stream.canPatchNative({serial:11,raw}),false,'changed protection cannot reuse the canvas');stream.compositorMasks='[]';
  const frame=await stream.frame({generation:1,width:1280,height:800,data_base64:'new',native_tiles:[{x:0,y:0,width:1,height:1,data_base64:encodePng(1,1,Buffer.from([1,2,3,255]))}]},'d',7);
  assert.equal(frame.kind,'tiles');assert.equal(stream.exact,false,'an opaque input echo does not make untouched lossy pixels exact');
 }finally{await stream.close()}
});
test('MP-08/MP-10/MP-11 tile-only refinement never treats its source serial as a full PNG',async()=>{
 const stream=new DisplayStream(binding,{now:()=>0,wait:async()=>{},encoder:{close:async()=>{}}});
 stream.sequence=1;stream.document_id='d';stream.previous={signature:'motion'};
 const source={generation:1,width:1280,height:800,native_exact:true,native_repair:true,data_base64:'serial',settled_verified:true,refinement_serial:3,repair_tiles:[{x:0,y:0,width:1,height:1,data_base64:encodePng(1,1,Buffer.from([1,2,3,255]))}]};
 try{assert.equal((await stream.frame(source,'d',1)).kind,'tiles');assert.equal(stream.exact,true);assert.equal(await stream.frame(source,'other',2),null,'a tile-only repair cannot bootstrap a different document');}finally{await stream.close()}
});
test('MD-DISPLAY unsafe process IDs rejected without signals', () => {
  for (const pid of [undefined, NaN, 0, 1, -1, -50, 2.5]) assert.throws(() => safeChildPid({ pid }));
  assert.equal(safeChildPid({ pid: 42 }), 42);
});
test('MD-DISPLAY video, exact settle, dirty patches and lost base recovery', async () => {
  const waits = [];
  const stream = new DisplayStream(binding, { encoder: { encode: async () => 'YWJj', close: async () => {} }, now: () => 0, wait: async ms => waits.push(ms) });
  const video = await stream.frame(fixture(), 'd', 0); assert.equal(video.kind, 'video');
  const exact = await stream.frame(fixture(), 'd', 1); assert.equal(exact.kind, 'png');
  assert.equal(await stream.frame(fixture(), 'd', 2), null);
  const patch = await stream.frame(fixture(0), 'd', 2); assert.equal(patch.kind, 'tiles'); assert.equal(patch.base_sequence, 2);
  assert.equal(patch.tiles.length, 1); assert.equal(decodePng(patch.tiles[0].data_base64).pixels[0], 0);
  const recover = await stream.frame(fixture(0), 'd', 100); assert.equal(recover.kind, 'video');
  const newDocument = await stream.frame(fixture(0), 'new-document', 4); assert.equal(newDocument.kind, 'video');
  stream.invalidate(); assert.equal((await stream.frame(fixture(), 'new-document', 5)).kind, 'video');
  assert.ok(waits.every(ms => ms > 0));
});
test('MD-DISPLAY DPR2 protected pixel masking scales coordinates', () => {
  const data = encodePng(2560, 1600, Buffer.alloc(2560 * 1600 * 4, 255));
  assert.throws(() => decodePng(data));
  const result = decodePng(maskPng(data, [[200, 200, 40, 40]], 2), 2);
  assert.equal(result.pixels[(210 * 2560 + 210) * 4], 0);
  assert.equal(result.pixels[(100 * 2560 + 100) * 4], 255);
});
test('MD-DISPLAY native crop tile bounds preserve all crossed tile edges', () => {
  const previous={width:512,height:256,pixels:Buffer.alloc(512*256*4,255)};
  const current={...previous,pixels:Buffer.from(previous.pixels)};
  const region={x:126,y:126,width:132,height:4};
  for(let y=region.y;y<region.y+region.height;y++)for(let x=region.x;x<region.x+region.width;x++)current.pixels[(y*512+x)*4]=0;
  const full=dirtyTiles(previous,current),cropped=dirtyTiles(previous,current,region);
  assert.equal(full.length,6);
  const restored=Buffer.from(previous.pixels);
  for(const tile of cropped){const decoded=decodePng(tile.data_base64);for(let row=0;row<tile.height;row++)decoded.pixels.copy(restored,((tile.y+row)*512+tile.x)*4,row*tile.width*4,(row+1)*tile.width*4)}
  assert.deepEqual(restored,current.pixels);
  assert.ok(cropped.every(tile=>tile.width<=32&&tile.height<=32));
  assert.deepEqual(dirtyTiles(current,current,region),[]);
});
test('MD-DISPLAY async encoder launch failure rejects into cleanup', async () => {
  const original = process.env.CHARIOX_BROWSER_DISPLAY_PYTHON;
  process.env.CHARIOX_BROWSER_DISPLAY_PYTHON = '/nonexistent/md-display-python';
  const encoder = new PortableEncoder();
  try {
    await assert.rejects(encoder.encode(fixture().data_base64, 2_000_000)); assert.equal(encoder.child, null);
    await assert.rejects(encoder.encode(fixture().data_base64, 2_000_000));
    assert.equal(encoder.child === null, true, 'failed encoder must not spawn an unawaited replacement');
  }
  finally { await encoder.close(); if (original === undefined) delete process.env.CHARIOX_BROWSER_DISPLAY_PYTHON; else process.env.CHARIOX_BROWSER_DISPLAY_PYTHON = original; }
});
test('MD-DISPLAY disabled commands fail before launching host Chromium', async () => {
  const { KernelBrowserHost } = await import('./kernel-browser-host.mjs');
  const host = new KernelBrowserHost('/unused-md-display-profile', { chromium: { start: async () => { throw new Error('must not launch'); } } });
  const original = process.env.CHARIOX_KERNEL_BROWSER_DISPLAY; delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;
  try {
    for (const op of ['display_subscribe','display_next','display_input']) await assert.rejects(host.request({op}), /experimental display disabled/);
  } finally { if(original !== undefined) process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=original; }
});
test('MD-DISPLAY DPR changes fence protected capture before sampling secret pixels', async () => {
  const { captureProtectedPage } = await import('./kernel-browser-pixels.mjs');
  const connection = { send: async method => {
    if (method === 'Target.getTargets') return { targetInfos: [{ type: 'page', targetId: 'page' }] };
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'page', loaderId: 'd' } } };
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 1 };
    if (method === 'Runtime.evaluate') return { result: { value: ['visible', 1, 800] } };
    throw new Error('unexpected CDP call');
  } };
  const browser = { ensureConnection: async () => connection, resolvePageTarget: async () => ({ connection, sessionId: 'page' }) };
  let captured = false;
  const masked = await captureProtectedPage(browser, { target_id: 'page' }, ['synthetic-sensitive-value'], [{ target_id: 'page', document_id: 'd' }], async () => { captured = true; throw Error('must not capture a mismatched source'); }, 2);
  assert.equal(captured, false);
  const frame = decodePng(masked, 2);
  assert.equal(frame.width, 2560); assert.equal(frame.height, 1600); assert.equal(frame.pixels[0], 0);
  const crop = decodePng(await captureProtectedPage(browser, { target_id:'page' }, ['synthetic-sensitive-value'], [{target_id:'page',document_id:'d'}], async()=>{assert.fail('racing crop must not capture');},2,{x:100,y:100,width:64,height:32,scale:1}),2);
  assert.equal(crop.width,128);assert.equal(crop.height,64);assert.equal(crop.pixels[0],0);
});
test('MD-DISPLAY pacing credit is bounded and accrued, never unbounded idle burst', async () => {
  let time=0;const waits=[];
  const stream=new DisplayStream(binding,{encoder:{encode:async()=> 'YWJj',close:async()=>{}},now:()=>time,wait:async ms=>{waits.push(ms);time+=ms}});
  await stream.frame(fixture(),'d',0);await stream.frame(fixture(),'d',1);
  time+=1000;await stream.frame(fixture(0),'d',2);assert.equal(waits.at(-1),0,'idle credit covers a small patch');
  // Accrual is capped at 16 KiB even after a long idle period.
  time+=1_000_000;const noise=randomBytes(256*256*4);
  const pacingStart=time;stream.codec='png';await stream.frame({generation:1,data_base64:encodePng(256,256,noise)},'d',3);
  assert.ok(stream.tokens<=16*1024);
  assert.ok(time-pacingStart>500,'a large random frame still pays its total byte budget after long idle');
});
test('MD-DISPLAY motion stays video until unchanged protected capture', async()=>{
 const stream=new DisplayStream(binding,{encoder:{encode:async()=> 'YWJj',close:async()=>{}},now:()=>0,wait:async()=>{}});
 assert.equal((await stream.frame(fixture(1),'d',0)).kind,'video');
 assert.equal((await stream.frame(fixture(2),'d',1)).kind,'video');
 assert.equal((await stream.frame(fixture(3),'d',2)).kind,'video');
 assert.equal((await stream.frame(fixture(3),'d',3)).kind,'png');
});
test('MD-DISPLAY unchanged motion pixels consume neither codec work nor credit bytes', async()=>{
 let encoded=0;
 const stream=new DisplayStream(binding,{encoder:{encode:async()=>{encoded++;return 'YWJj'},close:async()=>{}},now:()=>0,wait:async()=>{}});
 const source={generation:1,width:1280,height:800,motion:true,data_base64:'opaque-protected-jpeg'};
 assert.equal((await stream.frame(source,'d',0)).kind,'video');
 assert.equal(await stream.frame(source,'d',1),null);assert.equal(encoded,1);assert.equal(stream.exact,false);
});
test('MD-DISPLAY large exact repair is bounded, completes and invalidates on motion/lost base',async()=>{
 const pixels=randomBytes(512*512*4);for(let i=3;i<pixels.length;i+=4)pixels[i]=255;
 const source={generation:1,data_base64:encodePng(512,512,pixels)};
 const stream=new DisplayStream(binding,{encoder:{encode:async()=> 'YWJj',close:async()=>{}},now:()=>0,wait:async()=>{}});
 const actual=Buffer.alloc(pixels.length);
 assert.equal((await stream.frame(source,'d',0)).kind,'video');
 let patches=0;
 for(let i=0;i<50;i++){
  const frame=await stream.frame(source,'d',stream.sequence);
  if(!frame)break;
  assert.equal(frame.kind,'tiles');assert.equal(frame.base_sequence,frame.sequence-1);
  assert.ok(Buffer.byteLength(JSON.stringify(frame))*4/3+1024<1024*1024);
  for(const tile of frame.tiles){const p=decodePng(tile.data_base64);for(let y=0;y<p.height;y++)p.pixels.copy(actual,((tile.y+y)*512+tile.x)*4,y*p.width*4,(y+1)*p.width*4)}
  patches++;
 }
 assert.ok(patches>1);assert.equal(stream.exact,true);assert.deepEqual(actual,pixels);
 stream.invalidate();await stream.frame(source,'d',0);await stream.frame(source,'d',stream.sequence);
 assert.equal(stream.exact,false);
 const nextPixels=Buffer.from(pixels);nextPixels[0]^=255;
 assert.equal((await stream.frame({generation:1,data_base64:encodePng(512,512,nextPixels)},'d',stream.sequence)).kind,'video');
 assert.equal(stream.repair,null);
 assert.equal((await stream.frame(source,'d',0)).kind,'video');assert.equal(stream.repair,null);
});

test('MD-DISPLAY pipeline credit lag is bounded; encoder retains delta state and resets after repairs',async()=>{
 const resets=[];
 const stream=new DisplayStream(binding,{encoder:{encode:async(p,b,reset)=>{resets.push(reset);return {data_base64:'YWJj',key:reset}},close:async()=>{}},now:()=>0,wait:async()=>{}});
 const first=await stream.frame(fixture(1),'d',0);
 const second=await stream.frame(fixture(2),'d',0);
 assert.equal(first.key,true);assert.equal(second.key,false);assert.equal(second.kind,'video');
 await stream.frame(fixture(2),'d',0);
 assert.equal((await stream.frame({generation:1,data_base64:encodePng(256,256,randomBytes(256*256*4))},'d',0)).key,true);
 assert.deepEqual(resets,[true,false,true]);
 assert.equal(stream.acceptsCredit(stream.sequence+1),false);
 stream.sequence=20;assert.equal(stream.acceptsCredit(11),false);assert.equal(stream.acceptsCredit(12),true);
});

test('MD-DISPLAY input-overtaken verification drops before sequence commit and resets codec dependency',async()=>{
 const resets=[];const stream=new DisplayStream(binding,{encoder:{encode:async(p,b,reset)=>{resets.push(reset);return {key:reset,data_base64:'YWJj'}},close:async()=>{}},now:()=>0,wait:async()=>{}});
 assert.equal(await stream.frame(fixture(1),'d',0,async()=>false),null);
 assert.equal(stream.sequence,0);assert.equal(stream.previous,null);
 assert.equal((await stream.frame(fixture(2),'d',0)).sequence,1);assert.deepEqual(resets,[true,true]);
});

test('MD-DISPLAY an overtaken delta after a committed frame forces an independent next frame',async()=>{
 const resets=[];const stream=new DisplayStream(binding,{encoder:{encode:async(p,b,reset)=>{resets.push(reset);return {key:reset,data_base64:'YWJj'}},close:async()=>{}},now:()=>0,wait:async()=>{}});
 await stream.frame(fixture(1),'d',0);
 assert.equal(await stream.frame(fixture(2),'d',1,async()=>false),null);
 const recovered=await stream.frame(fixture(3),'d',1);
 assert.equal(recovered.sequence,2);assert.equal(recovered.key,true);assert.deepEqual(resets,[true,false,true]);
});

test('MP-11 a delta dropped by a failed frame build forces an independent next frame',async()=>{
 const resets=[];const stream=new DisplayStream(binding,{encoder:{encode:async(p,b,reset)=>{resets.push(reset);return {key:reset,data_base64:'YWJj'}},close:async()=>{}},now:()=>0,wait:async()=>{}});
 await stream.frame(fixture(1),'d',0);
 await assert.rejects(stream.frame(fixture(2),'d',1,async()=>{throw Error('stale document')}),/stale document/);
 const recovered=await stream.frame(fixture(3),'d',1);
 assert.equal(recovered.key,true);assert.deepEqual(resets,[true,false,true]);
 await stream.close();
});

test('MD-DISPLAY native small damage requires an exact contiguous source and retains exact RGB without video',async()=>{
 const {nativeDamageTiles}=await import('./kernel-browser-tiles.mjs');
 const raw={width:1280,height:800,format:'bgr0',pixels:Buffer.alloc(1280*800*4),damage:[4,4,12,12]};
 raw.pixels[(5*1280+5)*4]=17;raw.pixels[(5*1280+5)*4+1]=29;raw.pixels[(5*1280+5)*4+2]=43;
 const tiles=nativeDamageTiles(raw);assert.equal(tiles.length,1);
 const decoded=decodePng(tiles[0].data_base64);assert.deepEqual([...decoded.pixels.subarray((5*32+5)*4,(5*32+5)*4+4)],[43,29,17,255]);
 for(const damage of [[-1,0,2,2],[0,0,1280,800],[2,2,1,1],[0,NaN,1,2]])assert.equal(nativeDamageTiles({...raw,damage}),null);
 let encoded=0;const stream=new DisplayStream({...binding,device_scale_factor:1},{encoder:{encode:async()=>{encoded++;return 'YWJj'},close:async()=>{}},now:()=>0,wait:async()=>{}});
 try{
  assert.equal(stream.canPatchNative({serial:2,raw}),false);
  stream.exact=true;stream.sequence=1;stream.document_id='d';stream.previous={signature:'old'};stream.compositorSerial=1;
  assert.equal(stream.canPatchNative({serial:2,raw}),true);assert.equal(stream.canPatchNative({serial:3,raw}),false);
  const frame=await stream.frame({generation:1,width:1280,height:800,data_base64:'new',native_tiles:tiles},'d',1);
  assert.equal(frame.kind,'tiles');assert.equal(frame.base_sequence,1);assert.equal(encoded,0);assert.equal(stream.exact,true);
  const protectedSource=fixture(9);
  assert.equal((await stream.frame({...protectedSource,settled_verified:true},'d',2)).kind,'png','protected fallback after native patch has no cached RGB comparison base');
  await assert.rejects(stream.frame({generation:1,width:1280,height:800,data_base64:'newer',native_tiles:tiles},'newdoc',3),/native patch base/);
 }finally{await stream.close()}
});

test('MP-08/MP-10/MP-11 sparse native repair survives dropped captures only against the complete exact base',async()=>{
 const {nativeDamageTiles}=await import('./kernel-browser-tiles.mjs');
 const raw={width:1280,height:800,length:1280*800*4,format:'bgr0',nativeExact(){},readRegion(){assert.fail('Node must not read pixels')},damage:[0,0,1280,800],damage_tiles:[[0,0,32,32],[1248,768,1280,800]],base_serial:7};
 assert.equal(nativeDamageTiles(raw,true),true);
 assert.throws(()=>nativeDamageTiles(raw),/native sparse/);
 for(const damage_tiles of [[],Array(33).fill([0,0,32,32]),[[0,0,33,32]],[[0,0,32,801]],[[0,0,NaN,32]],[[32,0,0,32]]])assert.equal(nativeDamageTiles({...raw,damage_tiles},true),null);
 const stream=new DisplayStream({...binding,device_scale_factor:1},{encoder:{close:async()=>{}},now:()=>0,wait:async()=>{}});
 try{
  stream.previous={signature:'exact'};stream.compositorSerial=7;
  assert.equal(stream.canPatchNative({serial:12,raw}),false,'lossy canvas cannot authorize exact sparse reuse');
  stream.exact=true;
  assert.equal(stream.canPatchNative({serial:12,raw}),true,'captures 8–11 were never presented');
  assert.equal(stream.canPatchNative({serial:12,raw:{...raw,base_serial:8}}),false,'a source-admitted capture is not the presented exact base');
  stream.repair=[{}];assert.equal(stream.canPatchNative({serial:12,raw}),false);
 }finally{await stream.close()}
});

test('MD-DISPLAY pacing uses elapsed time during delayed timers and bounds its byte window',async()=>{
 let time=0;let calls=0;
 const stream=new DisplayStream({...binding,bitrate:500000},{encoder:{encode:async()=>({key:true,data_base64:'Y'.repeat(60000)}),close:async()=>{}},now:()=>time,wait:async ms=>{calls++;time+=ms+400}});
 try{
  await stream.frame(fixture(),'d',0);
  assert.ok(calls<=4,'absolute deadlines must account for delayed timers, rather than adding every planned wait');
  assert.ok(stream.tokens>=-16384&&stream.tokens<=16384);
 }finally{await stream.close()}
});

test('MD-DISPLAY same source recovery key is delivered before a later dependent delta',async()=>{
 const stream=new DisplayStream(binding,{encoder:{close:async()=>{}},now:()=>0,wait:async()=>{}});
 const source={generation:1,width:2560,height:1600,motion:true,data_base64:'native-signature',encoded:{key:true,data_base64:'key'}};
 try{assert.equal((await stream.frame(source,'d',0)).key,true);assert.equal((await stream.frame(source,'d',1)).key,true);assert.equal(stream.sequence,2);assert.equal((await stream.frame({...source,encoded:{key:false,data_base64:'delta'}},'d',2)).key,false)}finally{await stream.close()}
});

test('MD-DISPLAY legacy codec offers stay independent; dependencies require explicit admission',async()=>{
 for(const dependencies of [false,true]){
  const resets=[];const s=new DisplayStream({...binding,dependencies},{encoder:{encode:async(p,b,key)=>{resets.push(key);return{key,data_base64:'YWJj'}},close:async()=>{}},now:()=>0,wait:async()=>{}});
  try{assert.equal((await s.frame(fixture(1),'d',0)).key,true);assert.equal((await s.frame(fixture(2),'d',1)).key,!dependencies);assert.deepEqual(resets,[true,!dependencies])}finally{await s.close()}
 }
});

test('MD-DISPLAY replacement verified pixels retire queued tiles before exact commit',async()=>{
 const pixels=randomBytes(256*256*4);for(let n=3;n<pixels.length;n+=4)pixels[n]=255;
 const newer=Buffer.from(pixels);for(let n=0;n<newer.length;n+=4)newer[n]^=255;
 const image=value=>({generation:1,data_base64:encodePng(256,256,value),settled_verified:true,refinement_serial:value===pixels?1:2});
 const s=new DisplayStream(binding,{encoder:{encode:async()=> 'key',close:async()=>{}},now:()=>0,wait:async()=>{}});
 const restored=Buffer.alloc(pixels.length);
 const apply=frame=>{for(const tile of frame.tiles??[]){const p=decodePng(tile.data_base64);for(let y=0;y<p.height;y++)p.pixels.copy(restored,((tile.y+y)*256+tile.x)*4,y*p.width*4,(y+1)*p.width*4)}};
 try{
  await s.frame({...image(pixels),settled_verified:false},'d',0);
  apply(await s.frame(image(pixels),'d',s.sequence));assert.ok(s.repair?.length);
  for(let n=0;n<50;n++){const frame=await s.frame(image(newer),'d',s.sequence);if(frame)apply(frame);if(s.exact)break}
  assert.equal(s.exact,true);assert.equal(restored.equals(newer),true,"replacement snapshot must be reconstructed exactly");
 }finally{await s.close()}
});

test('MD-DISPLAY exact native damage can acknowledge input over a lossy base without certifying untouched pixels',async()=>{
 const raw={format:'bgr0',width:1280,height:800,pixels:Buffer.alloc(1280*800*4),damage:[0,0,16,16]};
 const stream=new DisplayStream({subscription_id:'s',tab_id:'t',generation:1,device_scale_factor:1,bitrate:2000000,codec:'avc1.420033'},{encoder:{encode:()=>{throw Error('small damage must bypass full encode')},close:async()=>{}},wait:async()=>{}});
 try{
  stream.sequence=1;stream.document_id='d';stream.previous={signature:'video'};stream.compositorSerial=1;
  assert.equal(stream.canPatchNative({serial:2,raw}),true);
  const frame=await stream.frame({generation:1,width:1280,height:800,data_base64:'patch',native_tiles:[{x:0,y:0,width:1,height:1,data_base64:'fixture'}]},'d',1);
  assert.equal(frame.kind,'tiles');assert.equal(stream.exact,false,'unrepaired lossy pixels must remain unverified');
 }finally{await stream.close()}
});

test('MD-DISPLAY verified prepared repair selection does no full-raster encoding on a credit',async()=>{
 const pixels=randomBytes(256*256*4);for(let n=3;n<pixels.length;n+=4)pixels[n]=255;
 const current={width:256,height:256,pixels};
 const data_base64=encodePng(256,256,pixels);
 const repair_tiles=dirtyTiles(null,current,null,true);
 const stream=new DisplayStream(binding,{encoder:{close:async()=>{}},now:()=>0,wait:async()=>{}});
 stream.document_id='d';stream.sequence=1;stream.previous={signature:'lossy-base'};
 stream.pixels.run=()=>{throw Error('credit cannot wait for background repair generation')};
 try{
  const frame=await stream.frame({generation:1,pixels:current,data_base64,settled_verified:true,refinement_serial:4,repair_tiles},'d',1);
  assert.equal(frame.kind,'tiles');assert.ok(stream.repair?.length);assert.equal(stream.exact,false);
 }finally{await stream.close()}
});

// MD-DISPLAY-04: source selection precedes recovery admission on the host.
test('MD-DISPLAY lost viewer base discards a selected delta before independent recovery',async()=>{
 const stream=new DisplayStream(binding,{encoder:{close:async()=>{}},now:()=>0,wait:async()=>{}});
 const source={generation:1,width:2560,height:1600,motion:true,data_base64:'selected',encoded:{key:false,data_base64:'delta'}};
 let reset=0;
 stream.producer={invalidate:()=>{reset++},close:async()=>{}};
 stream.sequence=20;stream.document_id='d';stream.previous={signature:'old'};
 try{
  assert.equal(await stream.frame(source,'d',0),null,'selected dependent packet cannot recover missing viewer references');
  assert.equal(stream.sequence,20);assert.equal(reset,1);
  const recovered=await stream.frame({...source,encoded:{key:true,data_base64:'key'}},'d',0);
  assert.equal(recovered.kind,'video');assert.equal(recovered.key,true);assert.equal(recovered.sequence,21);
 }finally{await stream.close()}
});
test('MD-DISPLAY lost viewer base starts large prepared exact recovery independently',async()=>{
 const pixels=randomBytes(256*256*4);for(let n=3;n<pixels.length;n+=4)pixels[n]=255;
 const current={width:256,height:256,pixels};
 const data_base64=encodePng(256,256,pixels),repair_tiles=dirtyTiles(null,current,null,true);
 const resets=[];
 const stream=new DisplayStream(binding,{encoder:{encode:async(p,b,key)=>{resets.push(key);return{key,data_base64:'key'}},close:async()=>{}},now:()=>0,wait:async()=>{}});
 stream.sequence=20;stream.document_id='d';stream.previous={signature:'missing-base'};
 const source={generation:1,pixels:current,data_base64,settled_verified:true,refinement_serial:4,repair_tiles};
 try{
  const recovered=await stream.frame(source,'d',0);
  assert.equal(recovered.kind,'video');assert.equal(recovered.key,true);assert.deepEqual(resets,[true]);
  assert.equal(stream.repair,null);assert.equal(stream.exact,false);
  const repair=await stream.frame(source,'d',21);assert.equal(repair.kind,'tiles');assert.equal(repair.base_sequence,21);
 }finally{await stream.close()}
});

// MP-08/MP-10: input echo may borrow one bounded burst; ordinary frames repay it.
test('MP-08/MP-10 rate-controlled video is never paced; exact repairs pay the link rate',async()=>{
 let now=0;const waits=[];
 const stream=new DisplayStream({...binding,bitrate:8000000},{now:()=>now,wait:async ms=>{waits.push(ms);now+=ms},encoder:{close:async()=>{}}});
 const source=n=>({motion:true,generation:1,width:1280,height:800,data_base64:String(n),encoded:{key:true,data_base64:'A'.repeat(90000)}});
 try{
  for(let n=1;n<=3;n++)await stream.frame(source(n),'d',n-1);
  assert.equal(now,0,'video ships as encoded; its encoder and the ACK gate bound bytes');assert.deepEqual(waits,[]);
  // A lossless scroll frame (native moves + WebP residuals) is budgeted by shiftFits.
  stream.exact=true;const shift={generation:1,width:1280,height:800,data_base64:'shift',motion:false,moves:[[0,0,1280,700,-100]],native_tiles:[{x:0,y:700,width:1280,height:100,format:'webp',data_base64:'A'.repeat(60000)}]};
  await stream.frame(shift,'d',3);assert.equal(now,0,'lossless scroll frames are not token paced');
  const repair={generation:1,width:1280,height:800,data_base64:encodePng(1280,800,Buffer.alloc(1280*800*4,7)),force_lossless:true};
  await stream.frame(repair,'d',4);assert.ok(now>0,'exact repair bytes are paced');
 }finally{await stream.close()}
});

test('MP-08/MP-10 native exact damage cannot disappear on a scheduling fingerprint collision',async()=>{
 const stream=new DisplayStream(binding,{now:()=>0,wait:async()=>{},encoder:{close:async()=>{}}});
 stream.sequence=1;stream.document_id='d';stream.previous={signature:'collision'};stream.exact=true;
 const source={generation:1,width:1280,height:800,data_base64:'collision',native_tiles:[{x:0,y:0,width:1,height:1,data_base64:encodePng(1,1,Buffer.from([17,18,19,255]))}]};
 try{const frame=await stream.frame(source,'d',1);assert.equal(frame?.kind,'tiles');assert.equal(frame.sequence,2);assert.equal(frame.tiles.length,1)}finally{await stream.close()}
});

test('MP-08/MP-10/MP-11 private encoder snapshot is stable across source mutation, reused exchanges and teardown',async()=>{
 const {mkdtemp,readFile,readdir,rm}=await import('node:fs/promises');const {tmpdir}=await import('node:os');const path=await import('node:path');
 const root=await mkdtemp(path.join(tmpdir(),'chariox-mp20-encoder-'));
 const previous=process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT;process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=root;
 const encoder=new PortableEncoder();const pixels=Buffer.alloc(128*128*4,0);const raw={width:128,height:128,format:'bgr0',length:pixels.length,pixels};
 try{
  const first=encoder.encodeStripes(raw,8000000,true);pixels.fill(255);const a=await first;
  assert.equal(a.stripes.length,8);assert(a.stripes.every(row=>row.key));
  const firstBytes=await readFile(path.join(root,a.packet.name));encoder.discard(a);
  const b=await encoder.encodeStripes(raw,8000000);
  assert.equal(b.stripes.length,8);assert(b.stripes.every(row=>row.reference_sequence===1));
  assert.notDeepEqual(await readFile(path.join(root,b.packet.name)),firstBytes);encoder.discard(b);
  const idle=await encoder.encodeStripes(raw,8000000);assert.deepEqual(idle.stripes,[]);
  pixels.fill(0);const recover=await encoder.encodeStripes(raw,8000000,true);assert(recover.stripes.every(row=>row.key));encoder.discard(recover);
  await encoder.close();assert.deepEqual(await readdir(root),[],'owned handoff and packet artifacts must settle');
 }finally{await encoder.close();if(previous===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT;else process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=previous;await rm(root,{recursive:true,force:true})}
});

test('MP-08 Retina input preserves a bounded fourfold physical sparse budget',async()=>{
 const stream=new DisplayStream({subscription_id:'s',tab_id:'t',bitrate:8000000,device_scale_factor:2,codec:'avc1.420033'},{encoder:{close:async()=>{}},now:()=>0,wait:async()=>{}});
 try{
  stream.previous={};stream.exact=false;stream.compositorSerial=10;stream.compositorMasks='[]';
  const tiles=Array.from({length:96},(_,n)=>{const x=n%32*32,y=Math.floor(n/32)*32;return [x,y,x+32,y+32]});
  const raw={nativeExact:async()=>{},format:'bgr0',width:2560,height:1600,length:2560*1600*4,pixels:Buffer.alloc(2560*1600*4),damage:[0,0,2560,1600],serial:11,base_serial:1,adjacent_damage_tiles:tiles,[displayMaskRegions]:[]};
  assert.equal(stream.canPatchNative({serial:11,raw}),true);
  assert.equal(stream.canPatchNative({serial:12,raw:{...raw,serial:12}}),false);
  assert.equal(stream.canPatchNative({serial:11,raw:{...raw,adjacent_damage_tiles:Array(129).fill([0,0,32,32])}}),false);
  assert.equal(stream.canPatchNative({serial:11,raw:{...raw,width:1280,height:800,length:1280*800*4,pixels:Buffer.alloc(1280*800*4)}}),false);
 }finally{await stream.close();}
});

test('MP-11 protected codec rejection bootstraps a bounded opaque base before exact repairs',async()=>{
 const width=600,height=600,pixels=randomBytes(width*height*4);
 for(let n=3;n<pixels.length;n+=4)pixels[n]=255;
 const source={generation:1,force_lossless:true,data_base64:encodePng(width,height,pixels),repair_tiles:[]};
 let retired=0,refinements=0;
 // Production wraps the native PortableEncoder in BrowserEncoder.
 const fallback=new PortableEncoder();
 Object.assign(fallback,{nativeRevision:17,nativeDeliveredRevision:17,nativeRetire:name=>{assert.equal(name,fallback.nativeSession);retired++;}});
 const stream=new DisplayStream({...binding,device_scale_factor:1,css_width:width,css_height:height},{now:()=>0,wait:async()=>{},encoder:new BrowserEncoder(null,'target',fallback)});
 stream.refiner={invalidate:()=>refinements++,close:async()=>{}};
 try{
  const first=await stream.frame(source,'d',0);
  assert.equal(first.kind,'png');assert.ok(JSON.stringify(first).length<1024*1024);
  const restored=decodePng(first.data_base64).pixels;
  const black=Buffer.alloc(restored.length);for(let n=3;n<black.length;n+=4)black[n]=255;
  assert.deepEqual(restored,black,'bootstrap exposes only opaque black');
  assert.equal(restored[0],0);assert.equal(restored[3],255);assert.equal(stream.exact,false);
  assert.ok(stream.repair?.length);assert.equal(retired,1,'bootstrap retires video certificates');assert.ok(refinements>0,'pending repairs of the replaced canvas retire');assert.equal(stream.encoder.nativeRevision,undefined);assert.equal(stream.encoder.nativeDeliveredRevision,undefined);
  for(let n=0;!stream.exact&&n<100;n++){
   const frame=await stream.frame(source,'d',stream.sequence);
   assert.equal(frame.kind,'tiles');assert.ok(JSON.stringify(frame).length<1024*1024);
   for(const tile of frame.tiles){const decoded=decodePng(tile.data_base64);for(let y=0;y<tile.height;y++)decoded.pixels.copy(restored,((tile.y+y)*width+tile.x)*4,y*tile.width*4,(y+1)*tile.width*4);}
  }
  assert.equal(stream.exact,true);assert.deepEqual(restored,pixels);
  stream.invalidate();assert.equal((await stream.frame(source,'other',stream.sequence)).kind,'png');assert.equal(stream.exact,false);
 }finally{await stream.close()}
});

// MP-08/MP-10/MP-11 (review #893 @8067044d1 P2): native raster::png() writes
// indexed PNGs for <=256 colours; an oversized one with no canvas base must
// still bootstrap black and repair every pixel exactly.
test('MP-11 an oversized indexed native PNG with no base bootstraps and repairs exactly',async()=>{
 const {deflateSync,crc32}=await import('node:zlib');
 const width=1280,height=800,indices=randomBytes(width*height),palette=Buffer.alloc(768);
 for(let n=0;n<256;n++){palette[n*3]=n;palette[n*3+1]=255-n;palette[n*3+2]=(n*37)&255;}
 const chunk=(type,body)=>{const head=Buffer.alloc(8);head.writeUInt32BE(body.length);head.write(type,4,'ascii');const tail=Buffer.alloc(4);tail.writeUInt32BE(crc32(Buffer.concat([head.subarray(4),body])));return Buffer.concat([head,body,tail]);};
 const ihdr=Buffer.alloc(13);ihdr.writeUInt32BE(width);ihdr.writeUInt32BE(height,4);ihdr[8]=8;ihdr[9]=3;
 const rows=Buffer.alloc((width+1)*height);for(let y=0;y<height;y++)indices.copy(rows,y*(width+1)+1,y*width,(y+1)*width);
 const png=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',ihdr),chunk('PLTE',palette),chunk('IDAT',deflateSync(rows)),chunk('IEND',Buffer.alloc(0))]);
 const expected=Buffer.alloc(width*height*4);for(let i=0;i<indices.length;i++){palette.copy(expected,i*4,indices[i]*3,indices[i]*3+3);expected[i*4+3]=255;}
 const source={generation:1,force_lossless:true,data_base64:png.toString('base64'),repair_tiles:[]};
 assert.ok(source.data_base64.length>1024*1024,'the exact indexed PNG exceeds bounded egress');
 const stream=new DisplayStream({...binding,device_scale_factor:1,css_width:width,css_height:height},{now:()=>0,wait:async()=>{},encoder:new BrowserEncoder(null,'target',new PortableEncoder())});
 try{
  const first=await stream.frame(source,'d',0);
  assert.equal(first.kind,'png');const restored=decodePng(first.data_base64).pixels;
  assert.ok(restored.every((v,i)=>i%4===3?v===255:v===0),'bootstrap exposes only opaque black');
  for(let n=0;!stream.exact&&n<400;n++){
   const frame=await stream.frame(source,'d',stream.sequence);
   assert.equal(frame.kind,'tiles');
   for(const tile of frame.tiles){const decoded=decodePng(tile.data_base64);for(let y=0;y<tile.height;y++)decoded.pixels.copy(restored,((tile.y+y)*width+tile.x)*4,y*tile.width*4,(y+1)*tile.width*4);}
  }
  assert.equal(stream.exact,true);assert.ok(restored.equals(expected),'every indexed pixel repaired exactly');
 }finally{await stream.close()}
});

test('MP-11 opaque bootstrap triggers only when the exact PNG exceeds bounded egress',async()=>{
 const width=400,height=400,pixels=randomBytes(width*height*4);
 for(let n=3;n<pixels.length;n+=4)pixels[n]=255;
 const source={generation:1,force_lossless:true,data_base64:encodePng(width,height,pixels),repair_tiles:[]};
 assert.ok(source.data_base64.length>700_000);
 const stream=new DisplayStream({...binding,device_scale_factor:1,css_width:width,css_height:height},{now:()=>0,wait:async()=>{},encoder:new BrowserEncoder(null,'target',new PortableEncoder())});
 try{
  const frame=await stream.frame(source,'d',0);
  assert.equal(frame.kind,'png');assert.ok(Buffer.from(decodePng(frame.data_base64).pixels).equals(pixels),'exact PNG ships unchanged');assert.equal(stream.exact,true);
 }finally{await stream.close()}
});
// MP-08/MP-10: lossless scroll selection needs an exact unprotected canvas on
// the compared base and residual bytes inside the budget; frames stay exact.
test('MP-08/MP-10 lossless scroll frames keep exactness and own their native packet',async()=>{
 const discarded=[],handed=[];
 const encoder={close:async()=>{},discard:e=>discarded.push(e),handedOff:e=>handed.push(e)};
 const stream=new DisplayStream({...binding,device_scale_factor:1,bitrate:8_000_000,codec:'avc1.420033'},{now:()=>0,wait:async()=>{},encoder});
 const plan={dy:-20,dirty_pixels:1920*20,moves:[[0,0,1920,1060]],dirty:[[0,1060,1920,20]]};
 const raw={nativeExact:async()=>{},width:1920,height:1080,serial:11,shift_adjacent:plan};
 stream.sequence=7;stream.document_id='d';stream.previous={signature:'exact'};stream.exact=true;stream.compositorSerial=10;stream.compositorMasks='[]';
 try{
  assert.equal(stream.shiftKind({serial:11,raw}),'adjacent');
  assert.equal(stream.shiftKind({serial:12,raw:{...raw,serial:12}}),'overlay','a dropped capture plans against the committed exact canvas');
  assert.equal(stream.shiftKind({serial:11,raw:{...raw,[displayMaskRegions]:[{x:0,y:0,width:1,height:1}]}}),null,'protected rasters stay on the guarded codec path');
  assert.equal(stream.shiftKind({serial:11,raw:{...raw,shift_adjacent:{...plan,dirty_pixels:1920*1000}}}),null,'a residual above a quarter second of link uses video');
  stream.shiftRate(895_000);assert.equal(stream.shiftKind({serial:11,raw}),null,'sustained residual traffic near the link uses video');stream.shiftBytes=0;
  stream.exact=false;assert.equal(stream.shiftKind({serial:11,raw}),null,'a lossy canvas cannot be a move source');stream.exact=true;
  stream.shiftHold=true;assert.equal(stream.shiftKind({serial:11,raw}),null,'a refused plan holds lossless frames');stream.shiftHold=false;
  assert.equal(stream.shiftKind({serial:10,raw:{...raw,serial:10}}),null,'the delivered serial needs no frame');
  stream.shiftRate(0,1920*1080*2.5);assert.equal(stream.shiftKind({serial:11,raw}),null,'sustained residual pixels (encode CPU) use video');stream.shiftArea=0;
  const packet={name:'b'.repeat(32)+'.json',length:6000};
  const frame=await stream.frame({generation:1,width:1920,height:1080,native_exact:true,native_tiles:[{x:0,y:1060,width:1920,height:20,format:'webp'}],moves:[[0,0,1920,1060,-20]],native_packet:packet},'d',7);
  assert.equal(frame.kind,'tiles');assert.deepEqual(frame.moves,[[0,0,1920,1060,-20]]);assert.equal(frame.native_packet,packet);
  assert.equal(stream.exact,true,'moves from an exact canvas plus exact residuals remain exact');
  assert.deepEqual(handed.at(-1),{packet});assert.ok(stream.shiftBytesPerPixel<.3&&stream.shiftBytesPerPixel>0,"wire bytes per residual pixel are learned");
  assert.equal(await stream.frame({generation:1,width:1920,height:1080,native_exact:true,native_tiles:[],moves:[[0,0,1920,1060,-20]],native_packet:packet},'d',8,async()=>false),null);
  assert.deepEqual(discarded.at(-1),{packet},'a refused frame retires its residual packet');
 }finally{await stream.close()}
});


test('MP-08/MP-10 peer96 pacing charges raw encrypted wire bytes and preserves peer94 pacing',async()=>{
 const source={generation:1,data_base64:encodePng(256,256,randomBytes(256*256*4))};
 const elapsed=[];
 for(const relay_binary of [false,true]){
  let clock=0;const stream=new DisplayStream({...binding,codec:'png',relay_binary},{now:()=>clock,wait:async ms=>{clock+=ms},encoder:{close:async()=>{}}});
  try{assert.equal((await stream.frame(source,'d',0)).kind,'png');elapsed.push(clock)}finally{await stream.close()}
 }
 assert.ok(elapsed[0]>100,'large baseline packet is paced');
 assert.ok(elapsed[1]<elapsed[0]*.8&&elapsed[1]>elapsed[0]*.7,'binary transport removes outer base64 cost only');
});
test('MP-08/MP-10/MP-11 identical source adoption keeps the worker committed base distinct',()=>{
 const stream=new DisplayStream({subscription_id:'s',tab_id:'t',device_scale_factor:1,codec:'avc1.420033',bitrate:8000000},{encoder:{close:async()=>{}}});
 Object.assign(stream,{previous:{},exact:true,compositorSerial:8,compositorCommittedSerial:7,compositorMasks:'[]'});
 const raw={nativeExact(){},format:'bgr0',length:1280*800*4,base_serial:6,adjacent_damage_tiles:[[0,0,32,32]],shift_adjacent:{dy:-10,dirty_pixels:12800,moves:[[0,0,1280,790]],dirty:[[0,790,1280,10]]},width:1280,height:800,[displayMaskRegions]:[]};
 assert.equal(stream.canPatchNative({serial:9,raw}),false,'an adjacent patch cannot silently discard the committed overlay');
 assert.equal(stream.shiftKind({serial:9,raw}),'overlay','overlay planning uses the last actual commit');
 stream.invalidate();assert.equal(stream.compositorCommittedSerial,null);
});
