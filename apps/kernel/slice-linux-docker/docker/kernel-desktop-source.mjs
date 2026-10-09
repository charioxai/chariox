// MP-08 / MP-11: protected desktop source for the shared XDamage/codec pipeline.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { NativePipe, NativeRaster } from './kernel-browser-native-pipe.mjs';
import { safeChildPid } from './kernel-browser-display.mjs';
import { displayMaskRegions } from './kernel-browser-pixels.mjs';
import { ProtectionGate, awaitPresented, measureBrowserProtection } from './browser-protection-regions.mjs';
export class DesktopSource {
  constructor(binding, policy) {
    this.binding=binding;this.policy=policy;this.listeners=new Set();this.closed=false;this.latest=null;this.held=null;
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
  async start() {
    const child=spawn(process.env.CHARIOX_BROWSER_DISPLAY_PYTHON||'/usr/bin/python3',
      ['-u',fileURLToPath(new URL('./kernel-browser-xshm.py',import.meta.url))],
      {env:this.binding.environment,stdio:['pipe','pipe','ignore']});this.child=child;
    const fail=()=>{void this.close().catch(()=>{});};child.on('error',fail);child.on('exit',fail);child.stdin.on('error',fail);
    safeChildPid(child);
    const raster=new NativeRaster();
    const pipe=new NativePipe(header=>{
      if(header.width!==this.binding.width||header.height!==this.binding.height||!Array.isArray(header.protected_regions)||header.protected_regions.length>4096||header.protected_regions.some(rect=>!Array.isArray(rect)||rect.length!==4||!rect.every(Number.isSafeInteger)||rect[0]<0||rect[1]<0||rect[2]<1||rect[3]<1||rect[0]+rect[2]>header.width||rect[1]+rect[3]>header.height))throw Error('MP-11: desktop raster protection/geometry');
    },(header,pixels)=>{
      if(this.closed)return;
      const raw=raster.apply(header,pixels);raw.format='bgr0';raw[displayMaskRegions]=header.protected_regions.map(([x,y,width,height])=>({x,y,width,height}));
      const sample={raw,serial:raw.serial,width:raw.width,height:raw.height,motion:true,data_base64:raw.signature};
      // Serial 0 withholds every kernel-browser window: publish at once.
      if(header.protection_serial===0)this.publish(sample);
      else if(header.protection_serial===this.gate.protectionSerial)this.held={sample,captured:header.captured_ms,serial:header.protection_serial};
    });
    child.stdout.on('data',bytes=>{try{pipe.push(bytes);}catch{fail();}});
    child.stdin.write(JSON.stringify({desktop:true,pid:process.pid,width:this.binding.width,height:this.binding.height,browser_protection:null,protection_serial:0,
      mask:this.policy.unknown||Boolean(this.policy.values.length||this.policy.targets.length),
      processes:await this.binding.ownedProcesses(),browser_processes:await this.binding.browserProcesses()})+'\n');
    for(let n=0;n<200&&!this.latest&&!this.closed;n++)await delay(10);
    if(!this.latest||this.closed){await this.close();throw Error('MP-08: protected desktop source unavailable');}
    void this.verify();
    return this;
  }
  publish(sample) {
    if(this.closed)return;
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
      if(held&&verified===held.serial)this.publish(held.sample);
      if(changed)await this.wake();
      // Unadopted (no browser, DevTools open, unbindable page): retry calmly.
      await delay(this.gate.protectionSerial?4:50);
    }
  }
  async wake() {
    try{
      const processes=await this.binding.ownedProcesses(),browser_processes=await this.binding.browserProcesses();
      if(!this.closed)this.child?.stdin.write(JSON.stringify({refresh:true,processes,browser_processes,browser_protection:this.gate.protection,protection_serial:this.gate.protectionSerial})+'\n');
    }catch{await this.close();}
  }
  async close() {
    this.closed=true;this.latest=null;this.listeners.clear();const child=this.child;this.child=null;
    if(!child)return;safeChildPid(child);child.stdin.end();
    for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await delay(20);
    if(child.exitCode===null&&child.signalCode===null)child.kill('SIGTERM');
    for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await delay(20);
    if(child.exitCode===null&&child.signalCode===null){child.kill('SIGKILL');await new Promise(resolve=>child.once('exit',resolve));}
  }
}
