// MP-08 / MP-10 / MP-11: typed configuration through the built TUI and hosted relay.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {stack,sleep} from './runtime.mjs';
import {clientModule,lane} from './paths.mjs';
import {pathToFileURL} from 'node:url';
import {shapedRemoteTui} from './remote-tui.mjs';
const {loadLocalKernelPresences,localKernelEndpoint}=await import(clientModule('local-kernel-presence.js'));
const {LOCAL_DAEMON_PROTOCOL_VERSION}=await import(pathToFileURL(lane+'/client/packages/kernel-client/dist/kernel-types.js').href);
const label=process.argv[2]??'921',name=process.argv[3]??'handoff-config';
const r=await stack(label,name,'');
const result={MP:['MP-08','MP-10','MP-11'],status:'FAIL',started:new Date().toISOString(),scope:'Config wire and restart; no provider turn or startup/latency/Path-1 acceptance'};
let remote;
const expected={codex_brief_model:'gpt-6-luna',claude_brief_model:'claude-haiku-5-5'};
function check(payload){
 const handoff=payload?.config?.history?.handoff;
 assert.deepEqual(handoff,expected);
 const canonical=Object.fromEntries(Object.entries(handoff).sort(([a],[b])=>a.localeCompare(b)));
 const sha256=createHash('sha256').update(JSON.stringify(canonical)).digest('hex');
 assert.equal(sha256,'40158e7f834b1d99b63ba7853672e2e5d4c671729965b78bdde2d9529919d1c3');
 return {handoff,sha256};
}
try{
 if(!process.env.CTXVAL_NATIVE_CODEX_PROFILE)throw Error('The real TUI requires a product-imported account status; supply CTXVAL_NATIVE_CODEX_PROFILE');
 const imported=await r.client.send({ImportNativeProviderAccountProfile:{provider:'codex'}});
 r.profiles.codex=imported.ProviderAccountProfile.profile.profile_id;
 await r.client.send({RefreshProviderAccountProfile:{provider:'codex',account_profile:r.profiles.codex}});
 await r.record('account-status-prerequisite',{profileId:r.profiles.codex,providerTurns:0,path:'Product import and explicit refresh, for normal TUI admission only'});
 remote=await shapedRemoteTui(r,{useTui:true});
 await r.launch('codex','gpt-6-luna');
 for(const [key,value]of Object.entries(expected)){
  await r.command('config set history.handoff.'+key+' '+value);
  let matched=false;
  for(let i=0;i<80;i++){const reply=await r.client.send({GetUserConfig:null});if(reply.UserConfig?.config?.history?.handoff?.[key]===value){matched=true;break;}await sleep(250);}
  assert(matched,'Visible config command did not persist '+key);
 }
 await r.capture('populated-handoff-config');
 await r.record('UserConfig',check((await r.client.send({GetUserConfig:null})).UserConfig));
 // Supplement the user writes with the actual mutation response, never recording other config fields.
 const updated=await r.client.send({SetUserConfigValue:{path:'history.handoff.codex_brief_model',value:expected.codex_brief_model}});
 await r.record('UserConfigUpdated',check(updated.UserConfigUpdated));
 const presence=loadLocalKernelPresences(r.env.CHARIOX_HOME+'/kernels/active').find(p=>localKernelEndpoint(p)===r.url);
 await r.record('wire-version',{client:LOCAL_DAEMON_PROTOCOL_VERSION,kernel:presence?.protocolVersion,expected:527});
 assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION,527,'Populated client config must advertise allocated protocol 527');
 assert.equal(presence?.protocolVersion,527,'Populated kernel config must advertise allocated protocol 527');
 await r.restart();
 await r.record('UserConfig-after-restart',check((await r.client.send({GetUserConfig:null})).UserConfig));
 await r.capture('populated-handoff-after-restart');
 result.status='PASS';result.persistedAcrossRestart=true;
}catch(error){result.firstSeam=String(error.message??error);await r.capture('handoff-config-failure').catch(()=>{});}
finally{result.finished=new Date().toISOString();await r.record('result',result);try{await r.close();}finally{await remote?.close();}}
if(result.status!=='PASS')process.exitCode=1;
