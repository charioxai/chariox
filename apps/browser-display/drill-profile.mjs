// MP-08/MP-10/MP-11: opt-in instruction samples of this drill's owned PIDs.
// No system-wide sampling, stacks, memory, command lines or environments.
import {mkdir,writeFile} from 'node:fs/promises';
import path from 'node:path';
import {launchOwned,stopGroup,waitChild,validPid} from './drill-owned-process.mjs';
export async function profileOwnedCpu(sample,output) {
 if(process.env.MD_CPU_PROFILE!=='1')return async()=>{};
 const dir=path.join(output,'cpu-profile');await mkdir(dir,{recursive:true});
 const jobs=[],receipt={MP:'MP-08/MP-10/MP-11',started_ms:Date.now(),groups:[]};
 try {
  for(const role of ['source','pipeline','viewer']) {
   const processes=sample.processes.filter(p=>p.role===role);
   if(!processes.length)continue;
   if(processes.some(p=>!validPid(p.pid)))throw Error('MP-11: invalid profile PID');
   const file=path.join(dir,role+'.data');
   const job=await launchOwned('/usr/bin/perf',['record','-q','--no-inherit','-e','cpu-clock','-F','499','-p',processes.map(p=>p.pid).join(','),'-o',file],{detached:true,stdio:'ignore'});
   jobs.push(job);receipt.groups.push({role,file,processes:processes.map(({pid,start,component})=>({pid,start,component}))});
  }
 }catch(error){for(const job of jobs)await stopGroup(job);throw error}
 return async()=>{
  for(const [i,job]of jobs.entries()){await stopGroup(job);receipt.groups[i].exit=await waitChild(job)}
  receipt.ended_ms=Date.now();await writeFile(path.join(dir,'receipt.json'),JSON.stringify(receipt,null,2)+'\n');
 };
}
