// G-07 owner identity: gate entries admitted under the local owner, then the kernel adopts an enrolled owner
// (startup enrollment re-owns sessions, agents, workflows). Does the enrolled owner still see, cancel and evaluate?
import {Rig,boot,submit,sleep,recorder,isTerminal,jq,runs,handoffs} from './lib.mjs';
import {writeFileSync,mkdirSync} from 'node:fs';
const rec=recorder('g07enroll');
const rig=new Rig('g07enroll');await rig.start();await rig.session();await boot(rig);const A=rig.alias;
const e1=await submit(rig,'run');await rig.waitFor(A,e1.entry_id,isTerminal,30000);
const e2=await submit(rig,'defer:300');await rig.waitFor(A,e2.entry_id,x=>x.disposition==='deferred',30000);
const before={owners:jq(rig,'select distinct owner_id from workflow_gate_entries'),mods:jq(rig,'select owner_id,count(*) from workflow_gate_modules group by 1'),state:jq(rig,'select owner_id,revision from workflow_gate_state'),entries:(await rig.allEntries(A)).map(e=>e.disposition)};
console.log('BEFORE',JSON.stringify(before));
await rig.stop();
const user='u-enrolled-drill';mkdirSync(rig.dir+'/home/daemon',{recursive:true,mode:0o700});
writeFileSync(rig.dir+'/home/daemon/config.json',JSON.stringify({cloud_relay:{api_url:'http://127.0.0.1:9',email:'drill@example.invalid',account_id:'acct-drill',user_id:user,account_slug:'drill',realm_id:'realm-drill',relay_url:'ws://127.0.0.1:9',issuer_id:'issuer-drill'}}),{mode:0o600});
rig.exited=undefined;await rig.start();
const room=(await rig.send({GetSessionState:{session_id:rig.sessionId}})).SessionState.session;
const after={roomOwner:room.owner_user_id,owners:jq(rig,'select distinct owner_id from workflow_gate_entries')};
let listed,listErr;try{listed=(await rig.allEntries(A)).map(e=>e.entry_id.slice(0,10)+':'+e.disposition)}catch(e){listErr=String(e).slice(0,200)}
let st,stErr;try{st=(await rig.state(A)).revision}catch(e){stErr=String(e).slice(0,200)}
let cancel;try{const r=await rig.cancel(A,e2.entry_id,'g07 enrolled cancel');cancel=JSON.stringify(r).slice(0,160)}catch(e){cancel='ERR '+String(e).slice(0,200)}
let fresh;try{const e3=await submit(rig,'run');const f=await rig.waitFor(A,e3.entry_id,isTerminal,30000);fresh={disp:f.disposition,failure:f.failure}}catch(e){fresh='ERR '+String(e).slice(0,300)}
await sleep(1500);
const final={deferredLeft:jq(rig,"select entry_id,disposition from workflow_gate_entries where disposition not in ('queued','dropped','failed','expired','cancelled')"),owners:jq(rig,'select owner_id,disposition,count(*) from workflow_gate_entries group by 1,2')};
const out={before,after,listed,listErr,st,stErr,cancel,fresh,final};console.log('OUT',JSON.stringify(out));
// defer:N re-evaluates every step, so the kept revision can only have advanced.
const revision=before.state.find(r=>r[0]==='local')?.[1];
const pass=Array.isArray(listed)&&listed.length===before.entries.length&&revision>0&&st>=revision&&!/^ERR/.test(cancel)&&final.owners.some(r=>r[1]==='cancelled')&&fresh?.disp==='queued'&&final.deferredLeft.length===0&&final.owners.every(r=>r[0]===user)&&after.owners.every(r=>r[0]===user);
rec('enrolled-owner-sees-pre-enrollment-gate-entries',{result:pass?'PASS':'FAIL',out});
await rig.stop();
