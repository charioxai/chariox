import {Rig,boot,submit,sleep,R,recorder,jq,runs,handoffs,settled,kinds,isTerminal,Checks,catchWorker,heldEval,sigWorker,journalSummary,findWorker} from './lib.mjs';
const rec=recorder('g05');const C=new Checks(rec);
let S;const rig=new Rig('g05');await rig.start();await rig.session();await boot(rig);
const A=rig.alias;
const done=(id,ms=60000)=>rig.waitFor(A,id,isTerminal,ms,'terminal');
const rev=async()=>(await rig.state(A)).revision;
const nruns=()=>handoffs(rig);
const nd=(id)=>jq(rig,'select count(*) from workflow_gate_decisions where entry_id=?',[id])[0][0];
async function restart(){await rig.kill9();rig.exited=undefined;await rig.start();}

// F7 kill right after admission (Accept -> Claim seam)
{const e0=await submit(rig,'run');await restart();const e=await done(e0.entry_id);await sleep(1500);
 C.ok('F7','SIGKILL right after admission: evaluated once after restart, one run',e.disposition==='queued'&&e.evaluation_count===1&&nd(e.entry_id)===1&&(S=await settled(rig)).ok,{disp:e.disposition,n:e.evaluation_count,db:nd(e.entry_id),S,acceptedState:e0.disposition});}
// F1 kill while Deferred
{const r0=await nruns();const e0=await submit(rig,'defer:30');
 const mid=await rig.waitFor(A,e0.entry_id,x=>x.disposition==='deferred',20000,'deferred');await restart();
 const e=await done(e0.entry_id,90000);await sleep(1500);const nums=e.decisions.map(d=>d.evaluation_number);
 C.ok('F1','SIGKILL while Deferred: resumes, one run, consecutive evaluation numbers',e.disposition==='queued'&&nums.every((n,i)=>n===i+1)&&e.evaluation_count===nums.length&&nd(e.entry_id)===nums.length&&(S=await settled(rig)).ok,{kinds:kinds(e),nums,S,beforeKill:mid.evaluation_count});}
// F2 kill mid-evaluation
{const r0=await nruns();const {entry:e0,worker:w}=await heldEval(rig,'run');const v0=await rev(); // after the hold: heldEval retries, and a missed attempt completes normally
 const ev=await rig.entry(A,e0.entry_id);
 await rig.kill9();sigWorker(w,'SIGKILL');rig.exited=undefined;await rig.start();
 const e=await done(e0.entry_id,60000);await sleep(3000);const e2=await rig.entry(A,e0.entry_id);
 C.ok('F2','SIGKILL mid-evaluation: Failed, never retried, state unchanged, no run',ev.disposition==='evaluating'&&e.disposition==='failed'&&e2.evaluation_count===e.evaluation_count&&e.evaluation_count===1&&(await rev())===v0&&(S=await settled(rig)).ok,{S,whileHeld:ev.disposition,disp:e.disposition,failure:e.failure,n:e2.evaluation_count,decisionKinds:kinds(e),revBefore:v0,revAfter:await rev(),hasReplayInput:!!e.decisions[0]?.evaluation});}
// F5 worker SIGKILLed, kernel alive
{const v0=await rev();const {entry:e0}=await heldEval(rig,'run','SIGKILL');const e=await done(e0.entry_id);
 const e1=await done((await submit(rig,'run')).entry_id);
 C.ok('F5','worker SIGKILL: entry Failed with worker error; next entry fine',e.disposition==='failed'&&/gate_/.test(e.failure||'')&&e1.disposition==='queued',{disp:e.disposition,failure:e.failure,revDelta:(await rev())-v0,next:e1.disposition});}
// F8 worker stopped past the 2 s total budget
{const {entry:e0,worker:w}=await heldEval(rig,'run');const e=await done(e0.entry_id,30000);await sleep(500);
 let alive=true;try{process.kill(w,0)}catch{alive=false}
 const e1=await done((await submit(rig,'run')).entry_id);
 C.ok('F8','stopped worker: bounded timeout, worker reaped, next entry fine',e.disposition==='failed'&&['gate_timeout','gate_interrupted','gate_worker_failed','gate_resource_limit'].includes(e.failure)&&!alive&&e1.disposition==='queued',{disp:e.disposition,failure:e.failure,workerAliveAfter:alive,next:e1.disposition});}
// F3 kill while Batched
{const r0=await nruns();const a=await submit(rig,'batch:20:fk');const b=await submit(rig,'batch:20:fk');
 const ma=await rig.waitFor(A,a.entry_id,x=>x.disposition==='batched',20000,'batched');const mb=await rig.waitFor(A,b.entry_id,x=>x.disposition==='batched',20000,'batched');
 const agg0=(await rig.allEntries(A)).filter(x=>x.disposition==='deferred'&&jq(rig,'select count(*) from workflow_gate_batches where entry_id=?',[x.entry_id])[0][0]===1);
 const flush=jq(rig,'select entry_id,flush_at_ms,closed from workflow_gate_batches');
 await restart();
 const t=Date.now();let agg;for(;;){const all=await rig.allEntries(A);agg=all.find(x=>x.entry_id===flush[0][0]);if(agg&&isTerminal(agg))break;if(Date.now()-t>90000)throw Error('batch not flushed');await sleep(500)}
 await sleep(1500);const members=jq(rig,'select count(*) from workflow_gate_members where group_id=(select group_id from workflow_gate_batches limit 1)')[0][0];
 const full=await rig.entry(A,agg.entry_id);const payload=full.decisions.at(-1)?.evaluation?.input?.batch;
 C.ok('F3','SIGKILL while Batched(2): one aggregate, 2 members, one run',agg.disposition==='queued'&&payload?.items?.length===2&&agg0.length===1&&(S=await settled(rig)).ok,{agg:agg.disposition,items:payload?.items?.length,members,S,flushRows:flush.length});}
// F4 cancel one batch member while the flush evaluation is active
{const r0=await nruns();const a=await submit(rig,'batch:40:fc');const b=await submit(rig,'batch:40:fc');
 await rig.waitFor(A,a.entry_id,x=>x.disposition==='batched',20000);await rig.waitFor(A,b.entry_id,x=>x.disposition==='batched',20000);
 const aggId=jq(rig,'select entry_id from workflow_gate_batches where closed=0 order by rowid desc limit 1')[0][0];
 const w=await catchWorker(rig,70000);sigWorker(w,'SIGSTOP');
 const held=await rig.entry(A,aggId);
 const c=await rig.cancel(A,a.entry_id,'wfgate cancel during flush');
 sigWorker(w,'SIGCONT');await sleep(2500);
 const agg=await done(aggId,30000);const mA=await rig.entry(A,a.entry_id);await sleep(1500);
 C.ok('F4','cancel member during active flush: aggregate Cancelled, no run',held.disposition==='evaluating'&&agg.disposition==='cancelled'&&mA.disposition==='cancelled'&&(S=await settled(rig)).ok,{held:held.disposition,agg:agg.disposition+'/'+agg.failure,member:mA.disposition+'/'+mA.failure,cancelResp:Object.keys(c),S});}
// F6 kill at the WaitingForQueue -> Queued handoff seam
{let hit=null,tries=0;for(;tries<15&&!hit;tries++){const e0=await submit(rig,'run');
   for(let i=0;i<400;i++){const e=await rig.entry(A,e0.entry_id);if(e.disposition==='waiting_for_queue'){hit=e0;break}if(isTerminal(e))break;await sleep(5)}
   if(hit){await restart();break}}
 if(hit){const e=await done(hit.entry_id,60000);const S2=await settled(rig);
  C.ok('F6','SIGKILL while WaitingForQueue: Queued after restart, exactly one handoff, no duplicates',e.disposition==='queued'&&S2.ok,{tries:tries+1,disp:e.disposition,S:S2});}
 else C.ok('F6','SIGKILL while WaitingForQueue (seam not reached)',false,{tries,note:'window shorter than poll interval'});}
const sum=journalSummary(rig);rec('journal-summary',{result:'INFO',sum});
await rig.stop();
console.log(JSON.stringify({failed:C.failed.length,total:C.rows.length}));
