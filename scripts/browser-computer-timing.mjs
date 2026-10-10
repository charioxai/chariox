// MP-08/MP-10/MP-11: opt-in drill instrumentation. Keep opaque packets opaque.
import {createHash} from 'node:crypto';
import {appendFileSync} from 'node:fs';

export function packetSignature(opcode,payload,withSequence=false) {
  try {
    let bytes,sequence;
    if(opcode===2){
      const wire=Buffer.from(payload,'base64');
      if(wire.length<8||wire.toString('ascii',0,4)!=='CXR1')return null;
      const length=wire.readUInt32BE(4);
      if(length<2||length>4096||wire.length<8+length+16)return null;
      const header=JSON.parse(wire.toString('utf8',8,8+length));
      if(!['daemon_event','client_event'].includes(header.kind))return null;
      bytes=wire.subarray(8+length);sequence=header.event_id;
    }else if(opcode===1){
      const envelope=JSON.parse(payload);
      if(!['client_request','daemon_request','daemon_event','client_event'].includes(envelope.kind))return null;
      const encrypted=envelope.encrypted_request??envelope.encrypted_event;
      if(typeof encrypted?.ciphertext!=='string')return null;
      bytes=Buffer.from(encrypted.ciphertext,'base64');sequence=envelope.event_id;
    }else return null;
    const packet=Number(createHash('sha256').update(bytes).digest().readBigUInt64BE(0)>>12n);
    return withSequence?{packet,...(Number.isSafeInteger(sequence)?{sequence}:{})}:packet;
  }catch{return null;}
}

export async function attachComputerTiming(page,file) {
  let pending=[],timer;
  const flush=()=>{clearTimeout(timer);timer=null;if(pending.length){appendFileSync(file,pending.join(''),{mode:0o600});pending=[];}};
  const record=row=>{pending.push(JSON.stringify(row)+'\n');if(pending.length>=256)flush();else if(!timer)timer=setTimeout(flush,16);};
  const cdp=await page.context().newCDPSession(page);
  await cdp.send('Network.enable');await cdp.send('Performance.enable');
  const before=Date.now(),{metrics}=await cdp.send('Performance.getMetrics'),after=Date.now();
  const clock=metrics.find(row=>row.name==='Timestamp')?.value;
  if(!Number.isFinite(clock))throw Error('MP-10 viewer monotonic clock unavailable');
  const offset=(before+after)/2-clock*1000;
  record({stage:'viewer_clock',at_ms:(before+after)/2,uncertainty_ms:(after-before)/2});
  for(const [event,stage]of [['Network.webSocketFrameSent','viewer_send'],['Network.webSocketFrameReceived','viewer_receive']])cdp.on(event,event=>{
    const packet=packetSignature(event.response.opcode,event.response.payloadData,true);
    if(packet!==null)record({stage,...packet,at_ms:offset+event.timestamp*1000,observed_ms:Date.now()});
  });
  await page.exposeBinding('__cuTimingRecord',(_,row)=>{
    if(!['viewer_input','viewer_paint'].includes(row?.stage)||!Number.isFinite(row.at_ms))return;
    record({stage:row.stage,at_ms:row.at_ms,...Object.fromEntries(['kind','sequence'].filter(key=>Number.isFinite(row[key])).map(key=>[key,row[key]]))});
  });
  await page.addInitScript(()=>{
    // No key, pointer coordinates, text, DOM content, IDs or pixels leave here.
    for(const [type,kind]of [['pointerdown',1],['keydown',2],['wheel',3]])document.addEventListener(type,event=>{
      if(event.isTrusted&&event.target?.closest?.('[role="dialog"]'))void globalThis.__cuTimingRecord({stage:'viewer_input',at_ms:performance.timeOrigin+event.timeStamp,kind});
    },true);
    const observe=()=>{
      const canvas=document.querySelector('canvas[aria-label="Selected kernel desktop"]');
      if(!canvas||canvas.__cuTimingObserved)return;
      canvas.__cuTimingObserved=true;
      new MutationObserver(()=>{const sequence=Number(canvas.dataset.displaySequence);requestAnimationFrame(()=>void globalThis.__cuTimingRecord({stage:'viewer_paint',at_ms:performance.timeOrigin+performance.now(),sequence}));}).observe(canvas,{attributes:true,attributeFilter:['data-display-sequence']});
    };
    new MutationObserver(observe).observe(document,{childList:true,subtree:true});
  });
  return {flush,async close(){flush();await cdp.detach().catch(()=>{});}};
}

export function transportHops(rows) {
  const packets=new Map();
  for(const row of rows)if(Number.isFinite(row.packet)&&Number.isFinite(row.at_ms)){
    const stages=packets.get(row.packet)??new Map();packets.set(row.packet,stages);
    const times=stages.get(row.stage)??[];stages.set(row.stage,times);times.push(row.at_ms);
  }
  const samples={viewer_to_relay:[],relay_to_kernel:[],kernel_queue:[],kernel_request:[],kernel_to_relay:[],relay_to_viewer:[],relay_write:[]};
  const span=(stages,key,from,to)=>{const start=stages.get(from)?.[0],end=stages.get(to)?.[0];if(Number.isFinite(start)&&Number.isFinite(end)&&end>=start)samples[key].push(end-start);};
  for(const stages of packets.values()){
    span(stages,'viewer_to_relay','viewer_send','relay_read');span(stages,'relay_to_kernel','relay_write_end','kernel_receive');
    span(stages,'kernel_queue','kernel_receive','kernel_request_start');span(stages,'kernel_request','kernel_request_start','kernel_request_end');
    span(stages,'kernel_to_relay','kernel_write_end','relay_read');span(stages,'relay_to_viewer','relay_write_end','viewer_receive');
    span(stages,'relay_write','relay_write_start','relay_write_end');
  }
  return Object.fromEntries(Object.entries(samples).map(([hop,values])=>{values.sort((a,b)=>a-b);return[hop,{samples:values.length,p50_ms:values[Math.floor(values.length*.5)]??null,p95_ms:values[Math.min(values.length-1,Math.floor(values.length*.95))]??null}];}));
}
