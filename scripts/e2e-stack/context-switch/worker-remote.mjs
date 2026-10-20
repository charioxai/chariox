import {acquireTerminationResource} from './termination.mjs';
import {registerTerminationCleanup} from './termination.mjs';
import {clientModule,playwrightModule} from './paths.mjs';
// MP-08 / MP-10 / MP-11: owned leased VM; SSH bridge is diagnostics and enrollment only.
const {chromium}=await import(playwrightModule());
import {spawnOwned,signalOwnedProcessGroup} from './owned.mjs';
import {writeFile,mkdir} from 'node:fs/promises';
import {sleep,lane,evidence,variant} from './runtime.mjs';
import net from 'node:net';
import {bootVm} from './vm.mjs';
import {bootDocker} from './docker-worker.mjs';
export async function remoteWorker(home,label,name,head){
 const diagnostic=process.env.CTXVAL_WORKER_KIND==='docker',vm=await (diagnostic?bootDocker(home,name):bootVm(home,name)),host=(diagnostic?'b3-Docker-diagnostic-':'b3-KVM-')+name;
 const socket=net.createConnection(vm.bridgePort,'127.0.0.1');await new Promise((resolve,reject)=>{socket.once('connect',resolve);socket.once('error',reject)});
 const child={stdout:socket,stdin:socket,stderr:{on(){}},on:(event,fn)=>socket.on(event==='exit'?'close':event,fn)};
 let serial=0,buffer='';const pending=new Map();let closed=false;
 child.stdout.setEncoding('utf8');child.stdout.on('data',chunk=>{buffer+=chunk;while(buffer.includes('\n')){const at=buffer.indexOf('\n'),line=buffer.slice(0,at);buffer=buffer.slice(at+1);let result;try{result=JSON.parse(line)}catch{continue}const p=pending.get(result.id);if(p){pending.delete(result.id);clearTimeout(p.timer);result.ok?p.resolve(result.data):p.reject(Error(result.error))}}});
 child.stderr.on('data',()=>{});child.on('exit',()=>{for(const p of pending.values()){clearTimeout(p.timer);p.reject(Error('remote diagnostic bridge exited'))}pending.clear()});
 function rpc(op,value){if(socket.destroyed||socket.readableEnded)return Promise.reject(Error('remote diagnostic bridge ended'));return new Promise((resolve,reject)=>{const id=++serial,timer=setTimeout(()=>{pending.delete(id);reject(Error('remote '+op+' deadline'))},360000);pending.set(id,{resolve,reject,timer});child.stdin.write(JSON.stringify({id,op,value})+'\n')})}
 const client={send:request=>rpc('request',request)};let identity;
 async function close(){if(closed)return;closed=true;try{const cleanup=await rpc('close');await home.record('remote-worker-cleanup',{...cleanup,host});}finally{socket.end();await vm.close();}}
 registerTerminationCleanup(close);
 try{
 identity=await rpc('start',{label,name,head});if(identity.head!==head)throw Error('Worker source head differs from requested exact candidate');await home.record('remote-worker-identity',identity);
 const origin=process.env.CTXVAL_ORIGIN,relay=origin.replace('https://','wss://').replace('.val.','relay.val.');
 const browser=await acquireTerminationResource(()=>chromium.launch({headless:true,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox','--disable-dev-shm-usage']}),browser=>browser.close());
 let status;
 try{const context=await browser.newContext(),page=await context.newPage();await page.goto(origin+'/validation/ready');const before=variant(await client.send({RelayStatus:null}),'RelayStatus').status;
 const login=variant(await client.send({StartCloudRelayLogin:{api_url:origin,client_id:'ctxfix2-r9-worker-'+name,client_alias:'Context worker '+name,machine_id:before.machine_id,machine_alias:'ctxfix2-worker-'+name}}),'CloudRelayLoginStarted').login;
 if(new URL(login.verification_url).origin!==origin)throw Error('worker device approval origin mismatch');await page.goto(login.verification_url);const approve=page.getByRole('button',{name:/approve|allow|connect/i});if(await approve.count()===1)await approve.click();let approved=false;
 for(let i=0;i<30;i++){await sleep(Math.max(5000,(login.interval_seconds??5)*1000));const state=variant(await client.send({PollCloudRelayLogin:{api_url:origin,device_code:login.device_code}}),'CloudRelayLoginPolled').result;if(state.status==='approved'){approved=true;break}if(state.status==='denied')throw Error('worker enrollment denied')}if(!approved)throw Error('worker enrollment timed out');await client.send({ConnectCloudRelay:null});
 for(let i=0;i<120;i++){status=variant(await client.send({RelayStatus:null}),'RelayStatus').status;if(status.connected)break;await sleep(500)}if(!status.connected||status.relay_url!==relay)throw Error('worker hosted WSS did not connect');
 let visible=false;for(let i=0;i<120;i++){visible=await page.evaluate(async target=>{const response=await fetch('/dashboard');if(!response.ok)throw Error('Worker dashboard status '+response.status);const data=await response.json();return data.relayTargets.some(t=>t.machineId===target.machine&&t.daemonId===target.kernel&&t.status==='ONLINE');},{machine:status.machine_id,kernel:status.daemon_id});if(visible)break;await sleep(500)}if(!visible)throw Error('New worker never became a fresh ONLINE product target');await home.record('remote-worker-visible',{machineId:status.machine_id,kernelId:status.daemon_id,online:true,source:'real hosted dashboard freshness policy'});
 }finally{await browser.close()}
 const enrollment={origin,relay,machine_id:status.machine_id,daemon_id:status.daemon_id,connected:true,host};await home.record('remote-worker-enrollment',enrollment);
 return {client,tree:identity.tree,root:identity.root,identity,enrollment,close,restart:()=>rpc('restart'),noteAbsent:name=>rpc('noteAbsent',name),nativeDiagnostics:()=>rpc('nativeDiagnostics'),admissionDiagnostics:()=>rpc('admissionDiagnostics'),resources:async()=>{await vm.check();return rpc('resources')},logs:()=>rpc('logObserve'),history:()=>rpc('sourceHistoryObserve')};
 }catch(e){await close().catch(()=>{});throw e}
}
