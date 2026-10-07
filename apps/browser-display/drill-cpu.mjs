// MP-08/MP-10: identical Linux CPU accounting for every display adapter.
import {readFile,readdir} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
export class CpuCounter {
 constructor(hz){this.hz=hz;this.entries=new Map();this.seconds={source:0,pipeline:0,viewer:0,harness:0};}
 observe(records){for(const r of records){const key=r.pid+':'+r.start;const previous=this.entries.get(key),ticks=Math.max(previous?.ticks??0,r.ticks);this.seconds[r.role]+=(ticks-(previous?.ticks??0))/this.hz;this.entries.set(key,{...r,ticks});}return this.totals();}
 totals(){return {...this.seconds};}
}
export function cpuSpan(before,after){const seconds=(after.at_ms-before.at_ms)/1000;const cores={};for(const role of ['source','pipeline','viewer','harness'])cores[role]=(after.cpu_seconds[role]-before.cpu_seconds[role])/seconds;return {duration_ms:seconds*1000,cores,source_plus_pipeline_cores:cores.source+cores.pipeline};}
export class CpuSampler {
 constructor({root,viewerRoot,harnessPid=process.pid}){this.root=root;this.viewerRoot=viewerRoot;this.harnessPid=harnessPid;this.counter=new CpuCounter(Number(execFileSync('getconf',['CLK_TCK'],{encoding:'utf8'})));this.known=new Map();this.components=new Map();this.programs=new Map();this.samples=[];this.roots=new Map();this.seen=new Set();}
 async track(pid,role='pipeline'){
  if(!Number.isSafeInteger(pid)||pid<=1||!['source','pipeline','viewer'].includes(role))throw Error('MP-10 invalid owned CPU root');
  const s=await readFile(`/proc/${pid}/stat`,'utf8'),f=s.slice(s.lastIndexOf(')')+2).split(' ');this.roots.set(pid,{start:f[19],role});
 }
 async sample(){
  const live=new Set((await readdir('/proc')).filter(n=>/^\d+$/.test(n)).map(Number));
  for(const pid of this.seen)if(!live.has(pid))this.seen.delete(pid);
  const tracked=new Set([...this.known.keys()].map(k=>Number(k.split(':')[0])));
  const inventory=[];
  for(const pid of live)if(tracked.has(pid)||this.roots.has(pid)||!this.seen.has(pid))try{
   const s=await readFile(`/proc/${pid}/stat`,'utf8'),f=s.slice(s.lastIndexOf(')')+2).split(' '),start=f[19];
   let role=this.known.get(pid+':'+start),root=this.roots.get(pid);
   if(root?.start===start)role=root.role;
   // MP-10: a source launcher may exec Chromium without changing PID/start.
   const key=pid+':'+start,program=s.slice(s.indexOf('(')+1,s.lastIndexOf(')')),changed=this.programs.get(key)!==program;
   const command=role&&!changed||root?.start===start?'':await readFile(`/proc/${pid}/cmdline`,'utf8');
   this.programs.set(key,program);
   if(pid===this.harnessPid)role='harness';
   else if(command.includes(this.viewerRoot))role='viewer';
   else if(command.includes(this.root))role=/chrome|chromium/.test(command.split('\0')[0])?'source':'pipeline';
   // MP-10: process names only, never raw command lines or environment values.
   let component=(changed?program:this.components.get(key))??program;
   if(component==='python3'){
    const args=command||await readFile(`/proc/${pid}/cmdline`,'utf8');
    for(const script of ['kernel-browser-xshm.py','kernel-browser-encoder.py'])if(args.split('\0').some(a=>a.endsWith('/'+script)))component=script;
   }
   this.components.set(pid+':'+start,component);
   inventory.push({pid,ppid:Number(f[1]),start,component,ticks:Number(f[11])+Number(f[12]),rss_bytes:Number(f[21])*4096});
   if(role)this.known.set(pid+':'+start,role);
  }catch{}
  const byPid=new Map(inventory.map(r=>[r.pid,r]));
  let changed=true;while(changed){changed=false;for(const r of inventory)if(!this.known.has(r.pid+':'+r.start)){const p=byPid.get(r.ppid),role=p&&this.known.get(p.pid+':'+p.start);if(role&&role!=='harness'){this.known.set(r.pid+':'+r.start,role);changed=true}}}
  this.seen=live;
  const records=inventory.filter(r=>this.known.has(r.pid+':'+r.start)).map(({ppid,...r})=>({...r,role:this.known.get(r.pid+':'+r.start)}));
  const sample={at_ms:performance.timeOrigin+performance.now(),cpu_seconds:this.counter.observe(records),processes:records.map(({ticks,...r})=>({...r,cpu_ticks:ticks}))};this.samples.push(sample);return sample;
 }
 start(){this.timer=setInterval(()=>{if(!this.pending)this.pending=this.sample().finally(()=>{this.pending=null})},100);this.timer.unref();}
 async close(){clearInterval(this.timer);await this.pending;return this.sample();}
}
