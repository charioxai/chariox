import {journalSummary,Rig,boot,submit,sleep,R,recorder,jq,runs,handoffs,settled,handoffSets,kinds,isTerminal,Checks} from './lib.mjs';
const rec=recorder('g04');const C=new Checks(rec);
const rig=new Rig('g04');await rig.start();await rig.session();await boot(rig);
const A=rig.alias;
const done=(id,ms=60000)=>rig.waitFor(A,id,isTerminal,ms,'terminal');
const rev=async()=>(await rig.state(A)).revision;
const nruns=async()=>{const h=await handoffSets(rig);return h.durableQueuedPrompts+h.runs};
const dbCount=(id)=>jq(rig,'select count(*) from workflow_gate_decisions where entry_id=?',[id])[0][0];

// D4 drop
{const r0=await nruns(),v0=await rev();const e=await done((await submit(rig,'drop')).entry_id);
 C.ok('D4','drop',e.disposition==='dropped'&&e.queued_prompt_id===null&&e.evaluation_count===1&&(await nruns())===r0&&(await rev())===v0+1,{disp:e.disposition,n:e.evaluation_count,kinds:kinds(e),revBefore:v0,revAfter:await rev()});}
// D1 defer:3
{const r0=await nruns();const e=await done((await submit(rig,'defer:8')).entry_id);const k=kinds(e);
 C.ok('D1','defer:8 then run',e.disposition==='queued'&&k.at(-1)==='run'&&k.slice(0,-1).every(x=>x==='defer_until')&&k.length>=3&&e.queued_prompt_id&&dbCount(e.entry_id)===e.evaluation_count,{kinds:k,n:e.evaluation_count,db:dbCount(e.entry_id)});
 const S1=await settled(rig);C.ok('D1','every Queued entry has exactly one handoff',S1.ok,S1);}
// D3 past
{const v0=await rev();const e=await done((await submit(rig,'past')).entry_id);
 C.ok('D3','defer to now -> gate_invalid_time',e.disposition==='failed'&&e.failure==='gate_invalid_time'&&e.evaluation_count===1&&(await rev())===v0,{disp:e.disposition,failure:e.failure,n:e.evaluation_count,revBefore:v0,revAfter:await rev()});}
// D5 faults
for(const [verb,allowed] of [['trap',['gate_trapped']],['spin',['gate_fuel_exhausted','gate_timeout','gate_resource_limit']],['badnode',['gate_invalid_target']]]){
 const r0=await nruns(),v0=await rev();const e=await done((await submit(rig,verb)).entry_id);
 C.ok('D5',verb,e.disposition==='failed'&&allowed.includes(e.failure)&&(await rev())===v0&&(await nruns())===r0,{disp:e.disposition,failure:e.failure,fuel:e.decisions[0]?.report?.fuel_consumed,ms:e.decisions[0]?.report?.execution_ms,revBefore:v0,revAfter:await rev()});}
{const e=await done((await submit(rig,'run')).entry_id);C.ok('D5','kernel still evaluates after faults',e.disposition==='queued',{disp:e.disposition});}
// D2 evaluation limit
{const r0=await nruns(),v0=await rev();const t=Date.now();const e=await done((await submit(rig,'forever')).entry_id,180000);
 C.ok('D2','forever -> gate_evaluation_limit at exactly 32',e.disposition==='failed'&&e.failure==='gate_evaluation_limit'&&e.evaluation_count===32&&dbCount(e.entry_id)===32&&(await nruns())===r0,{disp:e.disposition,failure:e.failure,n:e.evaluation_count,db:dbCount(e.entry_id),secs:(Date.now()-t)/1000,revDelta:(await rev())-v0});}
// D6 reset
{const q=await done((await submit(rig,'run')).entry_id);const pend=[];for(let i=0;i<3;i++)pend.push((await submit(rig,'defer:60')).entry_id);
 await sleep(1500);const mid=await Promise.all(pend.map(id=>rig.entry(A,id)));
 let bad1='none',bad2='none';try{const r=await rig.reset(A,'');bad1=JSON.stringify(r).slice(0,100)}catch(err){bad1='refused: '+String(err).slice(0,120)}
 try{const r=await rig.reset(A,'x'.repeat(513));bad2=JSON.stringify(r).slice(0,100)}catch(err){bad2='refused: '+String(err).slice(0,120)}
 C.ok('D6','empty / oversized reason refused',bad1.startsWith('refused')&&bad2.startsWith('refused'),{bad1,bad2});
 const snap=await rig.reset(A,'wfgate matrix reset');const after=await Promise.all(pend.map(id=>rig.entry(A,id)));const qa=await rig.entry(A,q.entry_id);
 C.ok('D6','pending entries Cancelled gate_state_reset; queued one untouched; state cleared',mid.every(x=>!isTerminal(x))&&after.every(x=>x.disposition==='cancelled'&&x.failure==='gate_state_reset')&&qa.disposition==='queued'&&snap.revision===0&&Object.keys(snap.state.workflow).length===0,{mid:mid.map(x=>x.disposition),after:after.map(x=>x.disposition+'/'+x.failure),queued:qa.disposition,snap});
 const ev=jq(rig,"select kind,payload_json from durable_state_events where kind='workflow.gate.state_reset'");
 C.ok('D6','reset event records actor+reason',ev.length===1&&ev[0][1].includes('wfgate matrix reset'),{rows:ev.length,payload:ev[0]?.[1]?.slice(0,300)});
 const e=await done((await submit(rig,'run')).entry_id);const st=await rig.state(A);
 C.ok('D6','entry after reset evaluates from 1',e.disposition==='queued'&&st.state.workflow.evaluated===1&&st.revision===1,{st});}
// D7 expiry: pending entries, restart with the clock 8 days ahead
{const late=(await submit(rig,'late')).entry_id;const soon=(await submit(rig,'defer:200')).entry_id;await sleep(2000);
 const b=await Promise.all([late,soon].map(id=>rig.entry(A,id)));
 C.ok('D7','pending before clock jump',b.every(x=>!isTerminal(x)),{b:b.map(x=>x.disposition+' due '+x.next_due_at_ms+' deadline '+x.deadline_ms)});
 const r0=await nruns();await rig.stop();rig.exited=undefined;
 Object.assign(rig.extraEnv,{LD_PRELOAD:R+'/shim/clockshim.so',FAKE_CLOCK_OFFSET_S:String(8*86400)});
 await rig.start();await sleep(4000);
 const a=await Promise.all([late,soon].map(id=>rig.entry(A,id)));
 C.ok('D7','after +8d clock: Expired gate_expired, no run',a.every(x=>x.disposition==='expired'&&x.failure==='gate_expired')&&(await nruns())===r0,{a:a.map(x=>x.disposition+'/'+x.failure),runsBefore:r0,runsAfter:await nruns()});
 const e=await done((await submit(rig,'run')).entry_id);C.ok('D7','new entry after jump evaluates',e.disposition==='queued'&&e.deadline_ms>Date.now()+8*86400000,{disp:e.disposition,deadline:e.deadline_ms});}
const sum=journalSummary(rig);rec('journal-summary',{result:'INFO',sum});
await rig.stop();
console.log(JSON.stringify({failed:C.failed.length,total:C.rows.length}));
