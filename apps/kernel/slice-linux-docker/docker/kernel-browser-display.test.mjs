import test from 'node:test';
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { DisplayStream, PortableEncoder, safeChildPid, dirtyTiles } from './kernel-browser-display.mjs';
import { encodePng, decodePng, maskPng } from './kernel-browser-pixels.mjs';
function fixture(value = 255) {
  const pixels = Buffer.alloc(256 * 256 * 4, 255); pixels[0] = value;
  return { generation: 1, data_base64: encodePng(256, 256, pixels) };
}
const binding = { subscription_id: 's', tab_id: 't', device_scale_factor: 2, bitrate: 2_000_000, codec: 'vp09.00.10.08' };
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
  stream.codec='png';await stream.frame({generation:1,data_base64:encodePng(256,256,noise)},'d',3);
  assert.ok(stream.tokens<=16*1024);
  assert.ok(waits.at(-1)>500,'a large random frame still pays its byte budget after long idle');
});
test('MD-DISPLAY motion stays video until unchanged protected capture', async()=>{
 const stream=new DisplayStream(binding,{encoder:{encode:async()=> 'YWJj',close:async()=>{}},now:()=>0,wait:async()=>{}});
 assert.equal((await stream.frame(fixture(1),'d',0)).kind,'video');
 assert.equal((await stream.frame(fixture(2),'d',1)).kind,'video');
 assert.equal((await stream.frame(fixture(3),'d',2)).kind,'video');
 assert.equal((await stream.frame(fixture(3),'d',3)).kind,'png');
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
