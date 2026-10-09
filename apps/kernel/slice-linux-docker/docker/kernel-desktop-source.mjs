// MP-08 / MP-11: protected desktop source for the shared XDamage/codec pipeline.
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { NativePipe, NativeRaster } from './kernel-browser-native-pipe.mjs';
import { NativeWorkerControl } from './kernel-browser-native-worker.mjs';
import { SharedRasterPool } from './kernel-browser-shared-raster.mjs';
import { safeChildPid } from './kernel-browser-display.mjs';
import { displayMaskRegions, maskNativeRaster } from './kernel-browser-pixels.mjs';
import { ProtectionGate, awaitPresented, measureBrowserProtection } from './browser-protection-regions.mjs';
const helper = name => fileURLToPath(new URL(name, import.meta.url));
const python = () => process.env.CHARIOX_BROWSER_DISPLAY_PYTHON || '/usr/bin/python3';
// MP-08/MP-10: the kernel's own native worker (XShm capture + x264), shared
// with the browser display, when this kernel was built with it.
export function nativeDesktopWorker(environment = process.env) {
  const executable = environment.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER, root = environment.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT;
  return path.isAbsolute(executable ?? '') && path.isAbsolute(root ?? '') ? { executable, root } : null;
}
const UNOBSERVED_MS = 200;
const validMasks = (masks, width, height) => Array.isArray(masks) && masks.length <= 4096 && masks.every(rect => Array.isArray(rect) && rect.length === 4 && rect.every(Number.isSafeInteger) &&
  rect[0] >= 0 && rect[1] >= 0 && rect[2] >= 1 && rect[3] >= 1 && rect[0] + rect[2] <= width && rect[1] + rect[3] <= height);
async function settle(child) {
  if (!child) return;
  safeChildPid(child); child.stdin.end();
  for (let n = 0; n < 50 && child.exitCode === null && child.signalCode === null; n++) await delay(20);
  if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
  for (let n = 0; n < 50 && child.exitCode === null && child.signalCode === null; n++) await delay(20);
  if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await new Promise(resolve => child.once('exit', resolve)); }
}
export class DesktopSource {
  constructor(binding, policy, { native = nativeDesktopWorker() } = {}) {
    this.binding=binding;this.policy=policy;this.native=native;this.listeners=new Set();this.closed=false;this.latest=null;this.held=null;
    // MP-08/MP-11: kernel-browser windows are revealed only under a stable,
    // presented CDP measurement that is re-verified after each frame's capture.
    this.gate=new ProtectionGate(async()=>{
      const browser=this.binding.browser?.();
      if(!browser||this.policy.unknown)return null;
      await awaitPresented(browser,this.gate.protection?.pages??[],1);
      return measureBrowserProtection(browser,this.policy);
    });
  }
  valid() { return !this.closed; }
  subscribe(fn) { this.listeners.add(fn);return()=>this.listeners.delete(fn); }
  sample() { return this.closed?null:this.latest; }
  async scope() {
    return { processes:await this.binding.ownedProcesses(), browser_processes:await this.binding.browserProcesses(),
      browser_protection:this.gate.protection, serial:this.gate.protectionSerial,
      mask:this.policy.unknown||Boolean(this.policy.values.length||this.policy.targets.length) };
  }
  async start() {
    try { if(this.native)await this.startNative();else await this.startHelper(); }
    catch(error) { await this.close(); throw error; }
    for(let n=0;n<200&&!this.latest&&!this.closed;n++)await delay(10);
    if(!this.latest||this.closed){await this.close();throw Error('MP-08: protected desktop source unavailable');}
    void this.verify();
    return this;
  }
  // Fallback without the native worker: the helper masks before its pipe.
  async startHelper() {
    const child=spawn(python(),['-u',helper('./kernel-browser-xshm.py')],{env:this.binding.environment,stdio:['pipe','pipe','ignore']});this.child=child;
    const fail=()=>{void this.close().catch(()=>{});};child.on('error',fail);child.on('exit',fail);child.stdin.on('error',fail);
    safeChildPid(child);
    const raster=new NativeRaster();
    const pipe=new NativePipe(header=>{
      if(header.width!==this.binding.width||header.height!==this.binding.height||!validMasks(header.protected_regions,header.width,header.height))throw Error('MP-11: desktop raster protection/geometry');
    },(header,pixels)=>{
      if(this.closed)return;
      const raw=raster.apply(header,pixels);raw.format='bgr0';raw[displayMaskRegions]=header.protected_regions.map(([x,y,width,height])=>({x,y,width,height}));
      this.offer({raw,serial:raw.serial,width:raw.width,height:raw.height,motion:true,data_base64:raw.signature},header.protection_serial);
    });
    child.stdout.on('data',bytes=>{try{pipe.push(bytes);}catch{fail();}});
    const scope=await this.scope();
    child.stdin.write(JSON.stringify({desktop:true,pid:process.pid,width:this.binding.width,height:this.binding.height,browser_protection:null,protection_serial:0,
      mask:scope.mask,processes:scope.processes,browser_processes:scope.browser_processes})+'\n');
  }
  // MP-08/MP-10/MP-11: the native worker reads the owned desktop root into
  // private slots and encodes with x264; every readback is masked here, bound
  // between two equal protection snapshots taken before and after it.
  async startNative() {
    const {width,height}=this.binding;
    this.current=await this.scope();
    this.poolRoot=await mkdtemp(path.join(this.native.root,'raster-'));
    const oracle=spawn(python(),['-u',helper('./kernel-desktop-protection.py')],{env:this.binding.environment,stdio:['pipe','pipe','ignore']});this.oracle=oracle;
    const child=spawn(this.native.executable,['--display-native-worker'],{env:this.binding.environment,stdio:['pipe','pipe','ignore']});this.child=child;
    const fail=()=>{void this.close().catch(()=>{});};
    for(const process of [oracle,child]){process.on('error',fail);process.on('exit',fail);process.stdin.on('error',fail);safeChildPid(process);}
    this.answers=createInterface({input:oracle.stdout})[Symbol.asyncIterator]();
    this.worker=new NativeWorkerControl(child);
    const pool=new SharedRasterPool(this.poolRoot,(slot,serial)=>this.worker.notify({release:slot,serial}));
    const pipe=new NativePipe(header=>{
      if(header.reply!==undefined){this.worker.validate(header);return;}
      if(header.width!==width||header.height!==height||!Number.isSafeInteger(header.serial)||header.serial<1||!Number.isFinite(header.captured_ms))throw Error('MP-11: desktop raster geometry');
    },(header,bytes)=>{
      if(header.reply!==undefined){this.worker.receive(header,bytes);return;}
      const raw=pool.apply(header);raw.format='bgr0';
      raw.nativeEncode=values=>this.worker.request('encode',{...values,serial:raw.serial});
      raw.nativeExact=values=>this.worker.request('exact',{...values,serial:raw.serial});
      raw.nativeRetire=encoder=>this.worker.retire(encoder);
      raw.nativeDelivered=(encoder,revision)=>this.worker.delivered(encoder,revision);
      raw.nativeCommit=(encoder,admit)=>this.worker.commit(encoder,raw.serial,admit);
      if(this.closed){raw.release();return;}
      this.next?.release();this.next=raw;void this.protect();
    });
    child.stdout.on('data',bytes=>{try{pipe.push(bytes);}catch{fail();}});
    child.stdin.write(JSON.stringify({pid:process.pid,width,height,pool:this.poolRoot,desktop:true})+'\n');
  }
  async measure(scope) {
    const start=Date.now();
    this.oracle.stdin.write(JSON.stringify({processes:scope.processes,browser_processes:scope.browser_processes,browser_protection:scope.browser_protection})+'\n');
    let timer;
    const next=await Promise.race([this.answers.next(),new Promise(resolve=>{timer=setTimeout(resolve,5000,{done:true});})]).finally(()=>clearTimeout(timer));
    if(next.done)throw Error('MP-11: desktop protection unavailable');
    const {digest,masks}=JSON.parse(next.value);
    if(typeof digest!=='string'||masks!==null&&!validMasks(masks,this.binding.width,this.binding.height))throw Error('MP-11: desktop protection reply');
    // Wall-clock ms like the worker's captured_ms; receipt follows completion.
    const snapshot={scope,digest,masks,start,end:Date.now()+1},now=Date.now();
    this.history=[...(this.history??[]).filter(item=>item.end>now-2000).slice(-63),snapshot];
    return snapshot;
  }
  // Snapshots run back to back while readbacks arrive. A readback is bound
  // between the last snapshot that ended before it began and one requested
  // after it arrived: all snapshots in that span must agree (else it is
  // masked whole). A span with an unobserved gap above UNOBSERVED_MS, or no
  // preceding snapshot, drops the readback and reads again.
  async protect() {
    if(this.protecting)return;this.protecting=true;
    try{while(this.next&&!this.closed){
      const raw=this.next,scope=this.current;this.next=null;
      if(scope.mask){this.bind(raw,null,scope.serial);continue;}
      try{await this.measure(scope);}catch(error){raw.release();throw error;}
      if(this.closed){raw.release();break;}
      const first=this.history.findLastIndex(item=>item.end<=raw.captured_ms),span=first<0?[]:this.history.slice(first);
      if(span.length<2||span.some((item,index)=>index&&item.start-span[index-1].end>UNOBSERVED_MS)){
        raw.release();if(!this.next)this.worker.notify({refresh:true},true);continue;
      }
      const stable=span.every(item=>item.scope===scope&&item.digest===span[0].digest);
      this.bind(raw,stable?span[0].masks:null,scope.serial);
    }}catch{this.next?.release();this.next=null;void this.close().catch(()=>{});}
    finally{this.protecting=false;}
  }
  bind(raw,masks,serial) {
    const {width,height}=this.binding;
    const regions=(masks??[[0,0,width,height]]).map(([x,y,w,h])=>({x,y,width:w,height:h}));
    const masked=maskNativeRaster(raw,regions,this.previousRegions,this.latest?.raw);
    if(masked!==raw)raw.release();
    this.previousRegions=regions;
    this.offer({raw:masked,serial:raw.serial,width,height,motion:true,data_base64:masked.signature},serial);
  }
  offer(sample,serial) {
    if(this.closed){sample.raw.release?.();return;}
    // Serial 0 withholds every kernel-browser window: publish at once.
    if(serial===0)this.publish(sample);
    else if(serial===this.gate.protectionSerial){this.held?.sample.raw.release?.();this.held={sample,serial};}
    else sample.raw.release?.();
  }
  publish(sample) {
    if(this.closed){sample.raw.release?.();return;}
    this.latest?.raw.release?.();
    this.latest=sample;
    for(const listener of this.listeners)listener(sample);
  }
  async verify() {
    while(!this.closed) {
      // Measuring holds the renderer's main thread (DOMSnapshot): only measure
      // to adopt protection or to verify a captured frame, never while idle.
      if(!this.held&&this.gate.protectionSerial!==0){await delay(8);continue;}
      // The latest frame captured before this measurement begins; frames
      // arriving meanwhile wait for the next one (no starvation while scrolling).
      const held=this.held;this.held=null;
      const {verified,changed}=await this.gate.step();
      if(held&&verified===held.serial)this.publish(held.sample);else held?.sample.raw.release?.();
      if(changed)await this.wake();
      // Unadopted (no browser, DevTools open, unbindable page): retry calmly.
      await delay(this.gate.protectionSerial?4:50);
    }
  }
  async wake() {
    try{
      const scope=await this.scope();
      if(this.closed)return;
      if(this.worker){
        // A new scope retires earlier snapshots; an unchanged one keeps them.
        const key=value=>JSON.stringify([value.processes,value.browser_processes,value.serial,value.mask]);
        if(key(scope)!==key(this.current))this.current=scope;
        this.worker.notify({refresh:true},true);return;
      }
      this.child?.stdin.write(JSON.stringify({refresh:true,processes:scope.processes,browser_processes:scope.browser_processes,browser_protection:scope.browser_protection,protection_serial:scope.serial})+'\n');
    }catch{await this.close();}
  }
  async close() {
    if(this.closed&&!this.child&&!this.oracle)return;
    this.closed=true;
    for(const raw of [this.latest?.raw,this.held?.sample.raw,this.next])raw?.release?.();
    this.latest=null;this.held=null;this.next=null;this.listeners.clear();
    const child=this.child,oracle=this.oracle,poolRoot=this.poolRoot;this.child=null;this.oracle=null;this.poolRoot=null;
    this.worker?.close();
    await Promise.all([settle(child),settle(oracle)]);
    if(poolRoot)await rm(poolRoot,{recursive:true,force:true});
  }
}
