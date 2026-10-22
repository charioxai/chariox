import {harness} from './paths.mjs';
import {clientModule,playwrightModule} from './paths.mjs';
// MP-08 / MP-10 / MP-11: remote diagnostic bridge; runtime traffic uses hosted WSS.
import {createInterface} from 'node:readline';
import {spawn,execFileSync} from 'node:child_process';
import {mkdir,writeFile,readFile,rm,statfs} from 'node:fs/promises';
import {createWriteStream} from 'node:fs';
import {createHash} from 'node:crypto';
import net from 'node:net';
import {admissionDiagnostics} from './admission-diagnostics.mjs';
const {LocalIpcClient}=await import(clientModule('ipc.js'));
import {pathToFileURL} from 'node:url';
const {createOwnedSignalGuard,processSnapshot}=await import(process.env.CTXVAL_SIGNAL_MODULE?pathToFileURL(process.env.CTXVAL_SIGNAL_MODULE).href:new URL('../../../apps/kernel/slice-linux-docker/owned-process-signals.mjs',import.meta.url).href);
const guard=createOwnedSignalGuard(),sleep=ms=>new Promise(r=>setTimeout(r,ms));let root,client,child,handle,env,identity;
let tree='/opt/drill/source';
function descendants(pid){const rows=processSnapshot(),ids=new Set([pid]);let changed=true;while(changed){changed=false;for(const row of rows)if(ids.has(row.ppid)&&!ids.has(row.pid)){ids.add(row.pid);changed=true}}return rows.filter(x=>ids.has(x.pid)&&x.pid>1)}
async function port(){return new Promise((resolve,reject)=>{const server=net.createServer();server.on('error',reject);server.listen(0,'127.0.0.1',()=>{const p=server.address().port;server.close(()=>resolve(p))})})}
async function start({label,name,head}){
 if(child)throw Error('worker already started');if(!/^\d{3}$/.test(label)||!/^[-\w]+$/.test(name))throw Error('invalid lane worker identity');root=(process.env.CTXVAL_STATE_ROOT??'/root/.chariox/dev/context-switch-worker')+'/worker-state-'+name;tree='/opt/drill/source';
 await mkdir(root,{recursive:true,mode:0o700});await writeFile(root+'/context-switch-owned.json',JSON.stringify({head,name}),{flag:'wx',mode:0o600});
 for(const p of ['home','os-home','tmp','config'])await mkdir(root+'/'+p,{recursive:true,mode:0o700});
 await writeFile(root+'/home/config.toml',`version = 1\n[history.archive]\nmode = "disabled"\n[credential_vault]\nbackend = "chariox_encrypted"\npath = "${root}/home/private-vault.json"\nunlock_policy = "kernel_init"\nagent_management = "allow"\n`,{mode:0o600});
 const ports=await Promise.all(Array.from({length:4},port));
 env={PATH:'/usr/local/bin:/usr/bin:/bin',LANG:'C.UTF-8',HOME:root+'/os-home',CHARIOX_HOME:root+'/home',TMPDIR:root+'/tmp',XDG_CONFIG_HOME:root+'/config',XDG_DATA_HOME:root+'/data',XDG_STATE_HOME:root+'/state',XDG_CACHE_HOME:root+'/cache',CHARIOX_KERNEL_PORT:String(ports[0]),CHARIOX_MCP_PORT:String(ports[1]),CHARIOX_CODEX_PORT:String(ports[2]),CHARIOX_OPENCODE_PORT:String(ports[3]),CHARIOX_DAEMON_SOCKET:root+'/daemon.sock',CHARIOX_KERNEL_RUNTIME_ROLE:'general',CHARIOX_ACCEPT_REMOTE_LEASES:'1',CHARIOX_REMOTE_LEASE_CAPACITY:'3',NODE_OPTIONS:'--max-old-space-size=4096',RUST_MIN_STACK:'16777216'};
 const binary='/opt/drill/kernel-'+label;if((await readFile('/opt/drill/ready-'+label,'utf8')).trim()!==head)throw Error('Worker binary ready source differs from requested head');if((await readFile('/opt/drill/source-head','utf8')).trim()!==head)throw Error('Worker worktree head differs from requested head');child=spawn(process.execPath,[tree+'/apps/kernel/slice-linux-docker/owned-process-anchor.mjs',binary],{cwd:tree,env,stdio:['ignore','pipe','pipe'],detached:true});if(!Number.isInteger(child.pid)||child.pid<=1)throw Error('invalid owned kernel pid');handle=guard.record(child.pid,{sessionLeader:true});const log=createWriteStream(root+'/kernel.private.log',{mode:0o600});child.stdout.pipe(log);child.stderr.pipe(log);
 const endpoint='ws://127.0.0.1:'+ports[0]+'/kernel';
 for(let i=0;i<120;i++){const c=new LocalIpcClient(endpoint,{timeoutMs:10000,localAuthEnvironment:env});try{await c.send({ListSessions:null});client=c;break}catch{await c.close();await sleep(500)}}if(!client)throw Error('remote kernel readiness failed');
 identity={root,tree,endpoint,binary,pid:child.pid,head,kernelSha256:createHash('sha256').update(await readFile(binary)).digest('hex'),host:process.env.CTXVAL_WORKER_HOST??'fresh-coordinator-worker'};return identity;
}
async function close({preserveState=false}={}){
 if(!child)return {alreadyClosed:true};const owned=descendants(child.pid),records=owned.map(p=>({p,handle:guard.pidHandle(handle,p.pid)}));
 if(client){await client.send({TeardownProviderProcesses:{provider:null,force:false}}).catch(()=>{});await client.close();client=null}
 if(child.exitCode===null&&child.signalCode===null)guard.group(handle,'SIGINT');await sleep(5000);
 for(const {p,handle:h} of records){if(!Number.isInteger(p.pid)||p.pid<=1)throw Error('unsafe pid');if(processSnapshot().some(x=>x.pid===p.pid&&x.start===p.start&&x.state!=='Z'))guard.pid(h,'SIGTERM')}
 await sleep(2000);for(const {p,handle:h} of records)if(processSnapshot().some(x=>x.pid===p.pid&&x.start===p.start&&x.state!=='Z'))guard.pid(h,'SIGKILL');await sleep(500);
 const remaining=processSnapshot().filter(p=>owned.some(x=>x.pid===p.pid&&x.start===p.start)&&p.state!=='Z');if(remaining.length)throw Error('owned remote processes remain');if(!preserveState)await rm(root,{recursive:true,force:true});child=null;return {remaining:[],removedState:preserveState?null:root,preservedState:preserveState,owned:owned.map(({pid,start,state})=>({pid,start,state}))};
}
async function restart(){
 if(!child||!identity)throw Error('No owned worker to restart');
 const before=identity,stopped=await close({preserveState:true});
 const binary=before.binary;if(createHash('sha256').update(await readFile(binary)).digest('hex')!==before.kernelSha256)throw Error('Owned worker binary changed before restart');
 child=spawn(process.execPath,[tree+'/apps/kernel/slice-linux-docker/owned-process-anchor.mjs',binary],{cwd:tree,env,stdio:['ignore','pipe','pipe'],detached:true});
 if(!Number.isInteger(child.pid)||child.pid<=1)throw Error('Invalid owned restarted kernel pid');handle=guard.record(child.pid,{sessionLeader:true});
 const log=createWriteStream(root+'/kernel.private.log',{flags:'a',mode:0o600});child.stdout.pipe(log);child.stderr.pipe(log);
 for(let i=0;i<120;i++){const c=new LocalIpcClient(before.endpoint,{timeoutMs:10000,localAuthEnvironment:env});try{await c.send({ListSessions:null});client=c;break}catch{await c.close();await sleep(500)}}if(!client)throw Error('Restarted worker readiness failed');
 await client.send({ConnectCloudRelay:null});let connected=false;
 for(let i=0;i<120;i++){const s=(await client.send({RelayStatus:null})).RelayStatus.status;if(s.connected){connected=true;break}await sleep(500)}
 if(!connected)throw Error('Restarted worker did not reconnect hosted relay');identity={...before,pid:child.pid};return {before,after:identity,stopped,connected:true,homeKernelRestarted:false};
}
const input=createInterface({input:process.stdin});
for await(const line of input){let item;try{item=JSON.parse(line);let data;
 if(item.op==='probe')data={ready:true};else if(item.op==='start')data=await start(item.value);else if(item.op==='request')data=await client.send(item.value);else if(item.op==='close')data=await close();else if(item.op==='restart')data=await restart();else if(item.op==='logObserve'){data=JSON.parse(execFileSync('python3',[harness+'/log-observe.py',root],{encoding:'utf8',env}));}else if(item.op==='sourceHistoryObserve'){data=JSON.parse(execFileSync('python3',[harness+'/history-worker-observe.py',root],{encoding:'utf8',env}));}else if(item.op==='nativeDiagnostics'){data=JSON.parse(execFileSync('python3',[harness+'/native-diag.py',root],{encoding:'utf8',env}));}else if(item.op==='admissionDiagnostics'){data=await admissionDiagnostics(root,client);}else if(item.op==='resources'){const fs=await statfs('/'),mem=await readFile('/proc/meminfo','utf8');data={memoryGiB:Number(mem.match(/^MemAvailable:\s+(\d+)/m)[1])/2**20,diskGiB:fs.bavail*fs.bsize/2**30};}else if(item.op==='noteAbsent'){if(!/^ctxval-[\w.-]+\.txt$/.test(item.value))throw Error('invalid note path');try{await readFile(root+'/audit-notes/'+item.value);data=false}catch(e){if(e.code!=='ENOENT')throw e;data=true}}else throw Error('unknown bridge operation');
 process.stdout.write(JSON.stringify({id:item.id,ok:true,data})+'\n');
 }catch(e){process.stdout.write(JSON.stringify({id:item?.id,ok:false,error:String(e?.message??e).replace(/Bearer\s+\S+|sk-ant-\S+/g,'[redacted]')})+'\n')}}
await close();
