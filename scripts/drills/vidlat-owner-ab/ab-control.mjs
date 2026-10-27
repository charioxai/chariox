// MP-08/MP-10/MP-11: normal product enrollment to the private fixture account.
import {readFile,writeFile,access} from 'node:fs/promises';import path from 'node:path';
import {LocalIpcClient} from './sdk/dist/ipc.js';
const root=process.env.AB_STATE,op=process.argv[2];
if(!/^\/root\/\.chariox\/dev\/vidlat\/owner-ab-[a-zA-Z0-9_-]+$/.test(root??''))throw Error('MP-11: own state required');
const state=JSON.parse(await readFile(root+'/runtime.json','utf8'));
const apiOrigin=state.apiOrigin??state.origin;
process.env.CHARIOX_HOME=root+'/V/runtime';
const c=new LocalIpcClient(`ws://127.0.0.1:${state.ports[0]}/kernel`);
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function prepareSession(){
 const workspace='/root/work/agent-vidlat-owner-ab-'+path.basename(root);
 if(await access(workspace).then(()=>true,()=>false))throw Error('MP-11: A/B workspace already exists');
 await writeFile(root+'/workspace.json',JSON.stringify({path:workspace}),{mode:0o600});
 await c.send({CreateWorkspaceDirectory:{path:workspace}});
 await writeFile(workspace+'/.vidlat-owner-ab','vidlat-owner-ab\n',{mode:0o600});
 const result=await c.send({CreateSession:{workspace_id:workspace,worktree_id:workspace,alias:'owner-ab',slice_ref:null}});
 const session=result.SessionCreated?.session;
 if(!session?.id)throw Error('MP-10: ordinary A/B session was not created');
 const selection=JSON.parse(await readFile(root+'/selection.json','utf8'));
 await writeFile(root+'/selection.json',JSON.stringify({...selection,sessionId:session.id,sessionAlias:'owner-ab'}),{mode:0o600});
 console.log('MP-08/MP-10/MP-11: ordinary owner-ab session prepared; no provider prompt dispatched.');
}
async function run(){try{
 if(op==='health'){
  let good=false;
  for(let n=0;n<120;n++){
   try{const h=(await c.send({GetDaemonHealth:null})).DaemonHealth.projection;if(h.process.process_id!==state.kernelPid)throw Error('MP-11: kernel identity mismatch');good=true;break}catch{await delay(500)}
  }
  if(!good)throw Error('MP-10: kernel readiness failed');
  console.log('MP-08/MP-10/MP-11: matching real kernel healthy');
 }else if(op==='login'||op==='rehearse-login'){
  const status=(await c.send({RelayStatus:null})).RelayStatus.status;
  const login=(await c.send({StartCloudRelayLogin:{api_url:apiOrigin,client_id:'vidlat-owner-ab',client_alias:'MP-08/MP-10 owner A/B',machine_id:status.machine_id,machine_alias:'vidlat-owner-ab CCX43'}})).CloudRelayLoginStarted.login;
  if(new URL(login.verification_url).origin!==state.origin)throw Error('MP-11: enrollment origin mismatch');
  // Device activation URL is retained locally for the product browser driver.
  // Never output a device code, token, credential or URL carrying one.
  await writeFile(root+'/device-login.private.json',JSON.stringify(login),{mode:0o600});
  console.log('MP-08/MP-10: device approval is pending in the private local enrollment record. Coordinator approves through the normal product device page using this stack’s disposable fixture identity.');
  for(let n=0;n<120;n++){
   await delay(Math.max(5000,(login.interval_seconds??5)*1000));
   const result=(await c.send({PollCloudRelayLogin:{api_url:apiOrigin,device_code:login.device_code}})).CloudRelayLoginPolled.result;
   if(result.status==='approved'){
    await c.send({ConnectCloudRelay:null});
    for(let retry=0;retry<180;retry++){
     const online=(await c.send({RelayStatus:null})).RelayStatus.status;
     const relayUrl=new URL(online.relay_url);
     if(relayUrl.protocol!=='wss:'||relayUrl.username||relayUrl.password||relayUrl.search||relayUrl.hash)throw Error('MP-11: Cloud supplied an unsafe hosted relay URL');
     if(state.relay&&online.relay_url!==state.relay)throw Error('MP-11: Cloud relay differs from the explicit expected URL');
     if(online.connected){
      await writeFile(root+'/selection.json',JSON.stringify({machineId:online.machine_id,daemonId:online.daemon_id,origin:state.origin,relay:online.relay_url}),{mode:0o600});
      await prepareSession();console.log('MP-08/MP-10: hosted WSS connected; select vidlat-owner-ab and owner-ab in the waiting room.');return;
     }await delay(500);
    }throw Error('MP-10: matching hosted WSS did not connect');
   }
   if(['denied','expired','expired_token'].includes(result.status))throw Error('MP-10: product device approval denied/expired');
  }throw Error('MP-10: device approval timed out');
 }else if(op==='session'){await prepareSession()}else throw Error('MP-11: supported operations are health/login/session');
}finally{await c.close()}}
await run();
