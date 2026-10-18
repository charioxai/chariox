// MP-08/MP-10/MP-11: one native readback/control adapter for page and window crops.
// The kernel resolves window geometry and admission; clients cannot name a PID,
// X window, shared file, mask policy or private worker configuration.
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {SharedRasterPool} from './kernel-browser-shared-raster.mjs';
import {NativePipe,NativeRaster} from './kernel-browser-native-pipe.mjs';
import {NativeWorkerControl,nativeWorkerResource} from './kernel-browser-native-worker.mjs';
import {ownsDisplay} from './kernel-browser-owned-display.mjs';
// MP-08/MP-10/MP-11: keep the worker's private fixed stage until the public error classifier.
export function nativeReadbackFailure(capture,message){
 return Object.assign(Error(message),{nativeStage:capture.helperStage});
}
export function startNativeReadback(capture,config){
 if(!ownsDisplay(capture.display)||!capture.valid()||!Number.isSafeInteger(capture.pid)||capture.pid<=1)
  throw Error('MP-11: native readback source not owned');
 if(config.window_region&&!nativeWorkerResource())
  throw Error('MP-08: cropped native worker unavailable');
 config=Object.freeze({...config,...(config.window_region?{window_region:Object.freeze({...config.window_region})}:{})});
   capture.phase='readback';const executable=nativeWorkerResource()?.executable;
   const child=spawn(executable||process.env.CHARIOX_BROWSER_DISPLAY_PYTHON||'python3',executable?['--display-native-worker']:['-u',fileURLToPath(new URL('./kernel-browser-xshm.py',import.meta.url))],{env:{...process.env,...capture.display.environment},stdio:executable?['pipe','pipe','pipe','pipe']:['pipe','pipe','pipe']});capture.child=child;if(executable)capture.nativeWorker=new NativeWorkerControl(child,capture.timing);
   const fail=()=>{if(!capture.closed)capture.timing?.(`native_readback_ended ${capture.surface??'page'}`,performance.timeOrigin+performance.now());capture.fence()};child.on('error',fail);child.on('exit',fail);child.stdin.on('error',fail);
   // MP-08/MP-10: fixed-label stage and owned-window sizes only.
   let stderr='';child.stderr.on('data',b=>{stderr+=b.toString();const lines=stderr.split('\n');stderr=lines.pop();if(stderr.length>8192)stderr='';for(const line of lines){
    if(/^MD-DISPLAY: native stage [a-z_]+$/.test(line))capture.helperStage=line.split(' ').at(-1);
    else if(/^MD-DISPLAY: native rejected (stale_document|epoch|region_revision|serial_skipped|shift_hold) \d+$/.test(line)){
     const [,reason,count]=line.match(/rejected (\w+) (\d+)$/);capture.timing.event?.('native_rejected',{reason:['stale_document','epoch','region_revision','serial_skipped','shift_hold'].indexOf(reason),count:Number(count),worker:child.pid});
    }
    else if(/^MD-DISPLAY: native geometry requested \d+x\d+ owned (none|\d+x\d+(,\d+x\d+){0,7})$/.test(line))capture.timing(line.replace('MD-DISPLAY: native geometry ','native_geometry '),performance.timeOrigin+performance.now());
   }});
   const raster=new NativeRaster();
   const pool=new SharedRasterPool(capture.poolRoot,(slot,serial)=>{capture.nativeWorker?.dropAuto(serial);capture.control({release:slot,serial})},config.window_region?{width:config.width,height:config.height}:null);
   const pipe=new NativePipe(header=>{
    if(header.windows_changed!==undefined){if(!Number.isFinite(header.windows_changed)||Object.keys(header).length!==2)throw Error('MD-DISPLAY: native windows header');return;}
    if(header.reply!==undefined){capture.nativeWorker?.validate(header);if(!capture.nativeWorker)throw Error('MP-11: unexpected native reply');return;}
    if(header.ended!==undefined){if(!capture.nativeWorker||Object.keys(header).length!==2||!Number.isSafeInteger(header.ended)||header.length!==1)throw Error('MP-11: native lease end');return;}
    if(header.generation!==undefined){if(!capture.nativeWorker||Object.keys(header).length!==3||!Number.isSafeInteger(header.auto)||!Number.isSafeInteger(header.generation)||!Number.isSafeInteger(header.length)||header.length<1||header.length>8*1024*1024)throw Error('MP-11: native auto reply');return;}
    if(header.auto!=null&&(!capture.nativeWorker||!Number.isSafeInteger(header.auto)))throw Error('MP-11: native auto frame');
    if(header.width!==config.width||header.height!==config.height||!Number.isSafeInteger(header.serial)||header.serial<1||!Number.isFinite(header.captured_ms)||!Number.isFinite(header.capture_ms)||!/^[a-f0-9]{16}$/.test(header.signature))throw Error('raw geometry');
   },(header,pixels)=>{
    if(header.windows_changed!==undefined){capture.windowsChanged?.();return;}
    if(header.reply!==undefined){capture.nativeWorker.receive(header,pixels);return;}
    if(header.generation!==undefined){capture.nativeWorker.autoReceive(header,pixels);return;}
    if(header.ended!==undefined){capture.nativeWorker.ended(header.ended);return;}
    const received=performance.timeOrigin+performance.now();
    if(Array.isArray(header.native_cpu)&&header.native_cpu.length===3)for(const [index,stage] of ['native_cpu_xshm_fence_read','native_cpu_damage_compare','native_cpu_capture_copy'].entries()){const cpu=header.native_cpu[index];if(Number.isFinite(cpu)&&cpu>=0)capture.timing(stage,header.captured_ms,header.captured_ms+cpu);}
    if(Number.isFinite(header.native_read_ms))capture.timing('native_capture_compare_copy',header.captured_ms,header.native_read_ms);
    // MP-08/MP-10: proved scroll plans offered by this readback (telemetry only).
    if(header.shift_adjacent)capture.timing('native_shift_adjacent',header.captured_ms,header.native_read_ms??header.captured_ms);
    for(const [name,start,end] of [['input_wake_to_capture',header.input_wake_ms,header.captured_ms],['native_damage_coalesce',header.damage_ready_ms,header.captured_ms],['native_window_fence',header.captured_ms,header.get_image_ms],['native_xshm_get_image',header.get_image_ms,header.image_ready_ms],['native_readback_copy',header.image_ready_ms,header.readback_ms],['native_readback',header.captured_ms,header.readback_ms],['native_fingerprint',header.readback_ms,header.fingerprint_ms],['native_damage_scan',header.fingerprint_ms,header.damage_ms]]){
     if(Number.isFinite(start)&&Number.isFinite(end)&&end>=start)capture.timing(name,start,end);
    }
    if(Number.isFinite(header.damage_ms))capture.timing('native_pipe',header.damage_ms,received);
    const at=performance.timeOrigin+performance.now(),raw=header.slot==null?raster.apply(header,pixels):pool.apply(header);capture.timing('native_raster_copy',at);
    if(capture.nativeWorker){
     raw.nativeEncode=values=>capture.nativeWorker.request('encode',{...values,serial:raw.serial});
     raw.nativeExact=values=>capture.nativeWorker.request('exact',{...values,serial:raw.serial});
     raw.nativeRetire=encoder=>capture.nativeWorker.retire(encoder);
     raw.nativeDelivered=(encoder,revision)=>capture.nativeWorker.delivered(encoder,revision);
     raw.nativeCommit=(encoder,admit)=>capture.nativeWorker.commit(encoder,raw.serial,admit);
     // L3: this readback was already encoded under state generation header.auto.
     if(header.auto!=null){const worker=capture.nativeWorker,serial=raw.serial;worker.autoEntry(serial);raw.auto={generation:header.auto,take:()=>worker.takeAuto(serial)};}
     raw.nativeArm=record=>capture.nativeWorker.arm(record);
     raw.nativeAutoCurrent=()=>capture.nativeWorker.auto;
     raw.nativeRetireState=encoder=>capture.nativeWorker.retireState(encoder);
     raw.nativeGrant=lease=>capture.nativeWorker.grant(lease);
     // MP-11: a lease needs no protected target anywhere (none can scroll into view).
     raw.nativeUntargeted=()=>capture.regions?.tracker?.().targets===false;
    }
    if(capture.valid()){raw.format='bgr0';if(capture.pending)capture.rejected('serial_skipped');capture.pending?.release?.();capture.pending=raw;void capture.publish()}else raw.release?.()
   });
   child.stdout.on('data',b=>{try{pipe.push(b)}catch{capture.fence()}});
   child.stdin.write(JSON.stringify({...config,pid:capture.pid,pool:capture.poolRoot,...(capture.nativeWorker?{retires:true,...(capture.nativeWorker.token?{frames:capture.nativeWorker.token}:{})}:{})})+'\n');
}
