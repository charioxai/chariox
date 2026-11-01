// MP-08/MP-10/MP-11: protocol 629 through the real kernel and encrypted relay.
// Run with the existing local display runner: MD_LAYOUT_PROTOCOL=1 node
// apps/browser-display/drill.mjs <kernel-test-ELF> <evidence> <node-tools> <py-tools>.
// This focused fixture proves the protocol contract, not public-site acceptance.
import assert from 'node:assert/strict';
import {inflateRawSync, constants as zlib} from 'node:zlib';
import {writeFile} from 'node:fs/promises';

export const layoutFixture = `<!doctype html><style>body{margin:0}p{margin:0;height:80px}</style>
<h1>Protocol 629 layout drill</h1>${Array.from({length:80},(_,i)=>`<p id="paragraph-${i}">Paragraph ${i} <b>deferred content</b></p>`).join('')}`;

function rows(nodes = []) {
 let serial=0;
 return nodes.map(row=>{serial+=row[0];return {id:`n${serial}`,attrs:row[3]??{},extra:row[4]??{}}});
}

export async function drillLayout({page,ready,output}) {
 assert(ready.protocol>=629,'MP-10: layout drill requires protocol 629');
 const request=async command=>{
  console.log(JSON.stringify({items:'MP-08/MP-10/MP-11',layout_op:command.op,...(command.op==='resize'?{width:command.width,height:command.height}:{})}));
  const reply=await page.evaluate(command=>mdTransport.request({KernelBrowser:{command}}),command);
  assert(!reply.Error,`MP-10: kernel refused ${command.op}: ${JSON.stringify(reply.Error)}`);
  assert(reply.KernelBrowser,`MP-10: missing KernelBrowser reply for ${command.op}`);
  return reply.KernelBrowser.result;
 };
 const binding={tab_id:ready.tab_id,generation:ready.generation},subscriptions=new Set();
 const subscribe=async extra=>{
  const binding=await request({op:'mirror_subscribe',tab_id:ready.tab_id,generation:ready.generation,wire:2,device_scale_factor:1,...extra});
  subscriptions.add(binding.subscription_id);
  let applied=0,context=Buffer.alloc(0);
  return {binding,next:async()=>{
   let packet=await request({op:'mirror_next',subscription_id:binding.subscription_id,generation:ready.generation,after_sequence:applied,drift_nodes:[],wait_ms:300});
   applied=packet.sequence;
   if(packet.reset)context=Buffer.alloc(0);
   if(packet.encoding==='deflate'){
    context=Buffer.concat([context,Buffer.from(packet.packet_base64,'base64')]);
    const inflated=inflateRawSync(context,{finishFlush:zlib.Z_SYNC_FLUSH});
    packet={...JSON.parse(inflated.subarray(inflated.length-packet.packet_bytes)),resources:packet.resources??[],tiles:packet.tiles??[]};
   }
   assert.equal(packet.wire,2,'MP-10: real mirror v2 packet');
   return packet;
  }};
 };
 const close=async viewer=>{
  await request({op:'mirror_close',subscription_id:viewer.binding.subscription_id,generation:ready.generation});
  subscriptions.delete(viewer.binding.subscription_id);
 };
 const geometry=(packet,width,height)=>assert.deepEqual([packet.css_width,packet.css_height],[width,height],'MP-10: shared page geometry');
 const capture=async(width,height,scale,label)=>{
  const shot=await request({op:'screenshot',...binding}),bytes=Buffer.from(shot.data_base64,'base64');
  assert.deepEqual([shot.width/scale,shot.height/scale],[width,height],'MP-10: screenshot uses the shared layout');
  assert.deepEqual([bytes.readUInt32BE(16),bytes.readUInt32BE(20)],[shot.width,shot.height],'MP-10: encoded PNG dimensions agree');
  await writeFile(`${output}/${label}-page.png`,bytes);
  await page.evaluate(async({shot,width,height})=>{
   const image=new Image();image.src='data:image/png;base64,'+shot.data_base64;await image.decode();
   const canvas=MDDisplay.canvas;canvas.width=shot.width;canvas.height=shot.height;canvas.style.width=width+'px';canvas.style.height=height+'px';
   canvas.getContext('2d').drawImage(image,0,0);
  },{shot,width,height});
  await page.screenshot({path:`${output}/${label}-viewer.png`});
 };
 const result={protocol:ready.protocol,transport:'real kernel + scoped encrypted relay + Chromium viewer',sizes:[],qualification:'Focused local protocol regression; no WAN, public-site, provider or stability acceptance'};
 try {
  const attached=await subscribe({});
  await attached.next();
  for(const [width,height] of [[900,600],[1920,1080],[400,700]]){
   const resized=await request({op:'resize',...binding,width,height});
   assert.deepEqual([resized.width,resized.height],[width,height],'MP-10: resize acknowledgement');
   const changed=await attached.next();
   assert.equal(changed.reset,true,'MP-10: attached viewer resets after resize');
   geometry(changed,width,height);
   await page.evaluate(async packet=>{
    const client=await import('/mirror-validator.mjs');
    const decoded={...packet,nodes:client.decodeMirror2Records(packet.nodes,null)};
    client.validateMirror2Packet(client.resolveMirror2Sheets(decoded,new Map()),new Map());
   },changed);
   // A protocol 560 observer cannot take layout ownership from a native resize.
   const legacy=await subscribe({layout_width:560,layout_height:480});
   geometry(await legacy.next(),width,height);
   await close(legacy);
   const later=await subscribe({});
   geometry(await later.next(),width,height);
   await close(later);
   await capture(width,height,changed.device_scale_factor,`layout-${width}x${height}`);
   result.sizes.push({width,height,attached_reset:true,legacy_observer_preserves_layout:true,layout_survives_close:true});
  }
  await close(attached);
  const viewport=await subscribe({viewport_first:true});
  assert.equal(viewport.binding.viewport_first,true,'MP-10: viewport-first negotiated');
  const first=await viewport.next();
  geometry(first,400,700);
  assert.equal(first.reset,true);
  assert.equal(first.snapshot_viewport_complete,true,'MP-10: visible viewport arrives first');
  assert.equal(first.snapshot_complete,false,'MP-10: off-screen subtrees are still pending');
  const deferred=rows(first.nodes).filter(row=>row.extra.deferred);
  assert(deferred.length>=40,'MP-10: off-screen paragraphs have placement boxes');
  const filled=new Set();let packet=first,credits=0;
  while(!packet.snapshot_complete&&credits++<200){
   packet=await viewport.next();
   for(const op of packet.ops??[])if(op.op==='children')filled.add(op.id);
  }
  assert.equal(packet.snapshot_complete,true,'MP-10: deferred snapshot completes');
  assert(deferred.every(row=>filled.has(row.id)),'MP-10: every deferred block receives its children');
  result.viewport_first={deferred:deferred.length,credits,complete:true};
  await close(viewport);
  await capture(400,700,first.device_scale_factor,'layout-final');
  return result;
 } finally {
  for(const subscription_id of subscriptions)await request({op:'mirror_close',subscription_id,generation:ready.generation}).catch(()=>{});
 }
}
