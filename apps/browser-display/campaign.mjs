// MD-DISPLAY-02/04: sequential campaign; never shape a shared interface.
import { execFileSync } from 'node:child_process';
import { readlink, readFile, writeFile, mkdir } from 'node:fs/promises';
import { randomBytes } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { launchOwned, waitChild, stopGroup, signalChild } from './drill-owned-process.mjs';
import { profiles } from './drill-netem.mjs';
const [binary,output,tools,pytools]=process.argv.slice(2);
if(![binary,output,tools,pytools].every(p=>p&&path.isAbsolute(p)))throw Error('MD-DISPLAY: four absolute paths required');
await mkdir(output,{recursive:true});
const cases=(process.env.MD_CASES||'local:docs,wan40:docs,wan80:docs,wan150:docs,local:canvas,local:video,local:scroll').split(',');
const manifest={item:'MD-DISPLAY-02/04',source:execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(),cases:[],started_at:new Date().toISOString()};
let active,interrupted=false;
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>{interrupted=true;signalChild(active,signal).catch(()=>{process.exitCode=1})});
try{
 for(const entry of cases){
  if(interrupted)break;
  const [profile,workload]=entry.split(':');if(!profiles[profile]||!['docs','canvas','video','scroll'].includes(workload))throw Error('MD-DISPLAY: unknown case');
  const namespace='md-display-'+randomBytes(6).toString('hex'),record={profile,workload,namespace};manifest.cases.push(record);
  let created=false;
  const ip=(...args)=>execFileSync('/usr/sbin/ip',args,{encoding:'utf8'});
  try{
   ip('netns','add',namespace);created=true;ip('netns','exec',namespace,'ip','link','set','lo','mtu','1500','up');
   // Disable segmentation/coalescing only inside the freshly-owned namespace.
   execFileSync('/usr/sbin/ip',['netns','exec',namespace,'ethtool','-K','lo','tso','off','gso','off','gro','off'],{stdio:'ignore'});record.mtu=1500;record.offloads='tso/gso/gro off';
   const budget=process.env.MD_ADAPT_BUDGET==='1'&&profiles[profile].mbps?Math.max(500000,Math.min(Number(process.env.MD_BITRATE||2000000),profiles[profile].mbps*500000)):Number(process.env.MD_BITRATE||2000000);record.bitrate=budget;
   const env={...process.env,MD_BITRATE:String(budget),MD_WORKLOAD:workload,MD_NETEM_PROFILE:profile,MD_HOST_NETNS:await readlink('/proc/self/ns/net')};
   active=await launchOwned('/usr/sbin/ip',['netns','exec',namespace,process.execPath,fileURLToPath(new URL('./drill.mjs',import.meta.url)),binary,path.join(output,entry.replace(':','-')),tools,pytools],{detached:true,stdio:'inherit',env});
   const exit=await waitChild(active);Object.assign(record,exit);
   if(exit.code!==0||exit.signal)process.exitCode=1;
  }catch(error){record.error=error.message;process.exitCode=1}
  finally{
   await stopGroup(active);active=null;
   if(created){const pids=ip('netns','pids',namespace).trim();record.remaining_namespace_pids=pids;
    if(pids){record.cleanup='RED: namespace retained; ownership uncertain';process.exitCode=1}
    else{ip('netns','delete',namespace);record.cleanup='owned namespace removed; process inventory empty'}
   }
   await writeFile(path.join(output,'campaign.json'),JSON.stringify(manifest,null,2));
  }
 }
}finally{if(interrupted)process.exitCode=130;manifest.finished_at=new Date().toISOString();manifest.exit_code=process.exitCode||0;await writeFile(path.join(output,'campaign.json'),JSON.stringify(manifest,null,2))}
