// MP-08/MP-10/MP-11: real presenter request ownership; supplementary regression.
import test from 'node:test';import assert from 'node:assert/strict';
import {attachBrowserDisplay} from './presenter.mjs';
test('MP-08/MP-10/MP-11 newer presented frames acknowledge without waiting for prior replies',async()=>{
 let listener;const acks=[];
 const transport={kernelProtocolVersion:536,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{}},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},
  request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?await new Promise(resolve=>acks.push({command,resolve})): {}}})};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1},{idleMs:20});
 stream.presenter.present=async frame=>{stream.presenter.sequence=frame.sequence;return true};
 try{
  for(const sequence of [1,2,3]){listener({event:'kernel_browser_frame',subscription_id:'s',frame:{sequence}});assert.equal((await stream.next()).sequence,sequence)}
  assert.deepEqual(acks.map(x=>x.command.sequence),[0,1,2,3],'ACK replies must not convoy newer acknowledgements');
 }finally{await stream.close();for(const {resolve}of acks)resolve({acknowledged:true,push:'running'})}
});

test('MP-08/MP-10/MP-11 90ms emulated wire RTT keeps presented ACK age within RTT+15ms',async t=>{
 t.mock.timers.enable({apis:['setTimeout','setInterval','Date'],now:0});
 let listener,sequence=0,source;const sent=new Map(),ages=[];
 const produce=()=>{const next=++sequence;sent.set(next,Date.now());setTimeout(()=>listener({event:'kernel_browser_frame',subscription_id:'s',frame:{sequence:next}}),45)};
 const transport={kernelProtocolVersion:536,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{}},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},
  request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?await new Promise(resolve=>{
   setTimeout(()=>{if(command.sequence===0&&!source){produce();source=setInterval(produce,33)}else if(sent.has(command.sequence))ages.push(Date.now()-sent.get(command.sequence));setTimeout(()=>resolve({acknowledged:true,push:'running'}),45)},45)
  }):{}}})};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1},{idleMs:20});
 stream.presenter.present=async frame=>{stream.presenter.sequence=frame.sequence;return true};
 try{
  for(let n=0;n<1500;n++){t.mock.timers.tick(1);for(let j=0;j<15;j++)await Promise.resolve()}
  const sorted=ages.sort((a,b)=>a-b),p95=sorted[Math.ceil(sorted.length*.95)-1];
  t.diagnostic(JSON.stringify({mp:['MP-08','MP-10','MP-11'],supplementary:true,emulatedRTT:90,ackAgeP95:p95,samples:ages.length}));
  assert(ages.length>=10);assert(p95<=105,JSON.stringify({mp:['MP-08','MP-10','MP-11'],supplementary:true,emulatedRTT:90,ackAgeP95:p95,samples:ages.length}));
 }finally{await stream.close();clearInterval(source);t.mock.timers.reset()}
});

test('MP-08/MP-10/MP-11 stalled ACK replies bound requests and retain the newest recovery request',async()=>{
 let listener;const acks=[];
 const transport={kernelProtocolVersion:536,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{}},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},
  request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?await new Promise(resolve=>acks.push({command,resolve})): {}}})};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1},{idleMs:20});
 stream.presenter.present=async frame=>{stream.presenter.sequence=frame.sequence;return true};
 try{
  for(let sequence=1;sequence<=20;sequence++){listener({event:'kernel_browser_frame',subscription_id:'s',frame:{sequence}});await stream.next()}
  assert.equal(acks.length,8,'even a peer that never replies has a bounded ACK window');
  stream.requestKey();assert.equal(acks.length,8);
  acks[0].resolve({acknowledged:true,push:'running'});
  for(let n=0;n<20;n++)await Promise.resolve();
  assert.equal(acks.length,9);assert.equal(acks.at(-1).command.sequence,20);assert.equal(acks.at(-1).command.lost,true);
  await stream.close();for(const {resolve}of acks)resolve({acknowledged:true,push:'running'});
  for(let n=0;n<20;n++)await Promise.resolve();assert.equal(acks.length,9,'retired streams cannot flush queued ACKs');
 }finally{await stream.close();for(const {resolve}of acks)resolve({acknowledged:true,push:'running'})}
});

test('MP-08/MP-10/MP-11 protocol536 ACK reports receiver hold separately from wire RTT',async()=>{
 let listener;const acks=[];
 const transport={kernelProtocolVersion:536,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{}},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},
  request:async({KernelBrowser:{command}})=>{if(command.op==='display_ack')acks.push(command);return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?{acknowledged:true,push:'running',ack_timing:true}:{}}}}};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1},{idleMs:20});
 stream.presenter.present=async frame=>{stream.presenter.sequence=frame.sequence;return true};
 try{
  const frame={sequence:1};Object.defineProperty(frame,Symbol.for('chariox.display.receivedAt'),{value:performance.timeOrigin+performance.now()-30});
  listener({event:'kernel_browser_frame',subscription_id:'s',frame});await stream.next();
  const held=acks.find(ack=>ack.sequence===1)?.held_ms;
  assert(Number.isSafeInteger(held)&&held>=30&&held<=100,'receiver hold must accompany its exact acknowledged frame');
  assert.equal(acks[0].held_ms,undefined,'the initial ACK has no received frame clock');
 }finally{await stream.close()}
});

for(const [protocol,capability]of [[491,true],[536,false]])test(`MP-08/MP-10/MP-11 protocol${protocol} capability${capability} keeps untimed peers' ACK shape`,async()=>{
 let listener;const acks=[];
 const transport={kernelProtocolVersion:protocol,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{}},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},
  request:async({KernelBrowser:{command}})=>{if(command.op==='display_ack')acks.push(command);return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?{acknowledged:true,push:'running',...(capability?{ack_timing:true}:{})}:{}}}}};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1},{idleMs:20});
 stream.presenter.present=async frame=>{stream.presenter.sequence=frame.sequence;return true};
 try{
  for(const sequence of [1,2]){listener({event:'kernel_browser_frame',subscription_id:'s',frame:{sequence}});await stream.next()}
  assert(acks.length>=3);assert(acks.every(ack=>!Object.hasOwn(ack,'held_ms')));
 }finally{await stream.close()}
});

// PERF_AUDIT 4.2.3 (protocol 540): a transport that holds frames pushed before its registration response
// asks for the first frame at registration (the 535 registration receive window presents it).
test('MP-08/MP-10 only a 540 holding push transport asks for its first frame at registration',async()=>{
 const subscribes=[];
 for(const [protocol,holds,expected]of[[540,true,true],[540,false,false],[536,true,false]]){
  const transport={kernelProtocolVersion:protocol,displayEventEncoding:'CXD1',initialPush:holds,onEvent:()=>()=>{},subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{},
   request:async({KernelBrowser:{command}})=>{if(command.op==='display_subscribe')subscribes.push(command);return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?{acknowledged:true,push:'running'}:{}}}}};
  const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1},{idleMs:20});
  try{assert.equal(subscribes.at(-1).initial_push===true,expected,`protocol ${protocol}, holds ${holds}`)}finally{await stream.close()}
 }
});
