import {acquireTerminationResource} from './termination.mjs';
// MP-08 / MP-10 / MP-11: dormant history comes from normal product device enrollment, never database injection.
import {mkdir,writeFile,rm} from 'node:fs/promises';
import net from 'node:net';
import {clientModule,playwrightModule,lane} from './paths.mjs';
import {spawnOwned,signalOwnedProcessGroup} from './owned.mjs';
import {createOwnedRuntime,sweepOwnedRuntime} from './runtime-owned.mjs';
import {sleep,variant,pins} from './runtime.mjs';
const {LocalIpcClient}=await import(clientModule('ipc.js'));
const {chromium}=await import(playwrightModule());
const port=()=>new Promise((resolve,reject)=>{const s=net.createServer();s.on('error',reject);s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p))})});
export async function enrollDormantHistory(run,count=65){
 if(!Number.isInteger(count)||count<65||count>80)throw Error('History reproduction requires65–80 real enrollments');
 const origin=process.env.CTXVAL_ORIGIN;if(!origin)throw Error('Hosted origin required');
 const browser=await acquireTerminationResource(()=>chromium.launch({headless:true,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox','--disable-dev-shm-usage','--js-flags=--max-old-space-size=1024']}),browser=>browser.close());
 const rows=[];
 try{const context=await browser.newContext(),page=await context.newPage();await page.goto(origin+'/validation/ready');
  for(let n=0;n<count;n++){
   const name='history-'+n,root=run.root+'/history/run-'+name;await createOwnedRuntime(root,{name,head:pins['921']});
   for(const p of ['home','os-home','tmp','config'])await mkdir(root+'/'+p,{recursive:true,mode:0o700});
   await writeFile(root+'/home/config.toml',`version = 1\n[credential_vault]\nbackend = "chariox_encrypted"\npath = "${root}/home/private-vault.json"\nunlock_policy = "kernel_init"\n`,{mode:0o600});
   const ports=await Promise.all(Array.from({length:4},port));
   const env={...run.env,CHARIOX_HOME:root+'/home',HOME:root+'/os-home',TMPDIR:root+'/tmp',XDG_CONFIG_HOME:root+'/config',XDG_DATA_HOME:root+'/data',XDG_STATE_HOME:root+'/state',XDG_CACHE_HOME:root+'/cache',CHARIOX_DAEMON_SOCKET:root+'/daemon.sock',CHARIOX_KERNEL_PORT:String(ports[0]),CHARIOX_MCP_PORT:String(ports[1]),CHARIOX_CODEX_PORT:String(ports[2]),CHARIOX_OPENCODE_PORT:String(ports[3])};
   delete env.CODEX_HOME;delete env.CHARIOX_RELAY_URL;delete env.CHARIOX_RELAY_TOKEN;
   const child=spawnOwned(lane+'/bin/kernel-921',[],{cwd:run.tree,env,stdio:'ignore',detached:true});let client;
   try{
    for(let attempt=0;attempt<120;attempt++){const c=new LocalIpcClient('ws://127.0.0.1:'+ports[0]+'/kernel',{timeoutMs:3000,localAuthEnvironment:env});try{await c.send({RelayStatus:null});client=c;break;}catch{await c.close();if(child.exitCode!==null||child.signalCode!==null)throw Error('History kernel exited before readiness');await sleep(250);}}
    if(!client)throw Error('History kernel readiness deadline');
    const before=variant(await client.send({RelayStatus:null}),'RelayStatus').status;
    const login=variant(await client.send({StartCloudRelayLogin:{api_url:origin,client_id:'ctx13-'+name,client_alias:'Context history '+n,machine_id:before.machine_id,machine_alias:'ctx13-dormant-'+n}}),'CloudRelayLoginStarted').login;
    if(new URL(login.verification_url).origin!==origin)throw Error('History approval origin mismatch');
    await page.goto(login.verification_url);const approve=page.getByRole('button',{name:/approve|allow|connect/i});
    // The normal device page can redirect as its approval settles. Only that
    // browser lifecycle race is retried; product polling still proves approval.
    for(let attempt=0;attempt<8;attempt++){try{if(await approve.count()===1)await approve.click();break;}catch(error){if(!/Execution context was destroyed|Cannot find context/.test(String(error))||attempt===7)throw error;await sleep(100);}}
    let approved=false;
    for(let attempt=0;attempt<24;attempt++){await sleep(Math.max(5000,(login.interval_seconds??5)*1000));const p=variant(await client.send({PollCloudRelayLogin:{api_url:origin,device_code:login.device_code}}),'CloudRelayLoginPolled').result;if(p.status==='approved'){approved=true;break;}if(p.status==='denied')throw Error('History product enrollment denied');}
    if(!approved)throw Error('History approval deadline');
    await client.send({ConnectCloudRelay:null});let connected=false;
    for(let attempt=0;attempt<120;attempt++){const p=variant(await client.send({RelayStatus:null}),'RelayStatus').status;if(p.connected){connected=true;break;}await sleep(250);}
    if(!connected)throw Error('History hosted relay registration deadline');
    rows.push({index:n,machineId:before.machine_id,kernelId:before.daemon_id,normalDeviceEnrollment:true,providerTurns:0});
   }finally{
    await client?.close();
    if(child.exitCode===null&&child.signalCode===null)signalOwnedProcessGroup(child,'SIGINT');
    for(let attempt=0;attempt<40&&child.exitCode===null&&child.signalCode===null;attempt++)await sleep(100);
    if(child.exitCode===null&&child.signalCode===null)signalOwnedProcessGroup(child,'SIGTERM');
    await sweepOwnedRuntime(root);await rm(root,{recursive:true,force:true});
   }
   await run.record('history-enrollment-'+n,{...rows.at(-1),runtimeRemoved:true});
  }
  const counts=await page.evaluate(async()=>{const response=await fetch('/dashboard');if(!response.ok)throw Error('Dashboard status '+response.status);const data=await response.json();const targets=data.relayTargets??[];return {nonRevoked:targets.filter(t=>t.status!=='REVOKED').length,online:targets.filter(t=>t.status==='ONLINE').length,offlineOrStale:targets.filter(t=>t.status==='OFFLINE'||t.status==='STALE').length};});
  await run.record('history-directory-count',counts);if(counts.nonRevoked<65)throw Error('Real enrolled history did not cross Cloud64target boundary');
  return {rows,counts};
 }finally{await browser.close();}
}
