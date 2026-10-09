// MP-08/MP-10/MP-11: sequential upstream baselines, same owned local netns setup.
import {execFileSync} from 'node:child_process';import {randomBytes} from 'node:crypto';
import {mkdir,writeFile,readFile} from 'node:fs/promises';import path from 'node:path';import {fileURLToPath} from 'node:url';
import {configureNamespace} from './drill-netns.mjs';
import {launchOwned,waitChild,stopGroup,signalChild} from './drill-owned-process.mjs';import {sourceIdentity} from './source-identity.mjs';
const [output,tools,selkies2,legacy]=process.argv.slice(2);
if(![output,tools,selkies2,legacy].every(p=>p&&path.isAbsolute(p)))throw Error('MP-10 four absolute paths required');
await mkdir(output,{recursive:true});const receipt={item:'MP-08/MP-10/MP-11',...await sourceIdentity(),cases:[],started_at:new Date().toISOString()};
let active,interrupted=false;for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>{interrupted=true;void signalChild(active,signal).catch(()=>{})});
const ip=(...args)=>execFileSync('/usr/sbin/ip',args,{encoding:'utf8'});
try{for(const backend of ['selkies2','legacy'])for(const workload of ['docs','canvas','video','scroll30','wheel30']){
 if(interrupted)break;
 const mem=Number((await readFile('/proc/meminfo','utf8')).match(/MemAvailable:\s+(\d+)/)[1])*1024;
 if(mem<Number(process.env.MD_MEMORY_FLOOR_GIB||12)*1024**3)throw Error('MP-10 memory floor');
 const namespace='md-display-'+randomBytes(6).toString('hex'),row={backend,workload,namespace,mem_available_bytes:mem,bitrate:8000000};receipt.cases.push(row);let created=false;
 try{ip('netns','add',namespace);created=true;configureNamespace(namespace);
  row.test_net_interface='unrouted192.0.2.2/24';row.mtu=1500;row.offloads='tso/gso/gro off';
  active=await launchOwned('/usr/sbin/ip',['netns','exec',namespace,process.execPath,fileURLToPath(new URL('./baseline-drill.mjs',import.meta.url)),path.join(output,backend+'-'+workload),tools,backend==='legacy'?legacy:selkies2],{detached:true,stdio:'inherit',env:{...process.env,MD_BASELINE:backend,MD_WORKLOAD:workload,MD_BITRATE:'8000000'}});
  Object.assign(row,await waitChild(active));if(row.code!==0||row.signal)process.exitCode=1;
 }catch(e){row.error=e.message;process.exitCode=1}
 finally{await stopGroup(active);active=null;if(created){row.remaining_namespace_pids=ip('netns','pids',namespace).trim();if(row.remaining_namespace_pids){row.cleanup='RED owned namespace retained';process.exitCode=1}else{ip('netns','delete',namespace);row.cleanup='owned namespace removed; process inventory empty'}}await writeFile(path.join(output,'campaign.json'),JSON.stringify(receipt,null,2));}
}}finally{receipt.exit_code=process.exitCode||0;receipt.finished_at=new Date().toISOString();await writeFile(path.join(output,'campaign.json'),JSON.stringify(receipt,null,2));}
