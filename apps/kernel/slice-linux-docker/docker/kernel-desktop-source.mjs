// MP-08 / MP-11: protected desktop source for the shared XDamage/codec pipeline.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { NativePipe, NativeRaster } from './kernel-browser-native-pipe.mjs';
import { safeChildPid } from './kernel-browser-display.mjs';
import { displayMaskRegions } from './kernel-browser-pixels.mjs';
export class DesktopSource {
  constructor(binding, policy) { this.binding=binding;this.policy=policy;this.listeners=new Set();this.closed=false;this.latest=null; }
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
      this.latest={raw,serial:raw.serial,width:raw.width,height:raw.height,motion:true,data_base64:raw.signature};
      for(const listener of this.listeners)listener(this.latest);
    });
    child.stdout.on('data',bytes=>{try{pipe.push(bytes);}catch{fail();}});
    child.stdin.write(JSON.stringify({desktop:true,pid:process.pid,width:this.binding.width,height:this.binding.height,
      mask:this.policy.unknown||Boolean(this.policy.values.length||this.policy.targets.length),
      processes:await this.binding.ownedProcesses(),browser_processes:await this.binding.browserProcesses()})+'\n');
    for(let n=0;n<200&&!this.latest&&!this.closed;n++)await delay(10);
    if(!this.latest||this.closed){await this.close();throw Error('MP-08: protected desktop source unavailable');}
    return this;
  }
  async wake() {
    try{
      const processes=await this.binding.ownedProcesses(),browser_processes=await this.binding.browserProcesses();
      if(!this.closed)this.child?.stdin.write(JSON.stringify({refresh:true,processes,browser_processes})+'\n');
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
