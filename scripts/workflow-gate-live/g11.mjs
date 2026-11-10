import {Rig,editConfig,boot,submit,sleep,R,recorder,jq,runs,kinds,isTerminal,Checks,catchWorker,heldEval,sigWorker,journalSummary,settled,wfSource} from './lib.mjs';
const rec=recorder('g11');const C=new Checks(rec);let S;
const rig=new Rig('g11');await rig.start();await rig.session();const b=await boot(rig,{config:{label:'v1',step_ms:300}});
const A=rig.alias;const done=(id,ms=90000)=>rig.waitFor(A,id,isTerminal,ms,'terminal');
const edit=async(label)=>editConfig(rig,{config:{label,step_ms:300}});
const rev=async()=>(await rig.state(A)).revision;
// C1 burst + mid-burst edit
{const ids=[];for(let i=0;i<14;i++)ids.push((await submit(rig,i%2?'run':'defer:6')).entry_id);
 await sleep(2500);const ed=await edit('v2');
 const post=[];for(let i=0;i<3;i++)post.push((await submit(rig,'run')).entry_id);
 const all=await Promise.all([...ids,...post].map(id=>done(id)));
 const hashes=new Set(all.map(e=>e.config_sha256));
 const byCfg={};for(const e of all){(byCfg[e.config_sha256]??=[]).push(e)}
 const [h1,h2]=[...hashes];
 // classify v1 (admitted before edit) vs v2 by admission time
 const v1=all.filter(e=>e.accepted_at_ms<ed.t0),v2=all.filter(e=>e.accepted_at_ms>=ed.t1);
 const mixed=all.filter(e=>e.decisions.some(d=>d.evaluation&&d.evaluation.configuration.config_sha256!==e.config_sha256));
 const staleAfterEdit=v1.filter(e=>e.decisions.some(d=>d.evaluation&&d.evaluation.input.evaluation_at_ms>ed.t1)&&e.disposition!=='cancelled'&&e.disposition!=='failed');
 const v1Pending=v1.filter(e=>e.disposition==='cancelled');
 C.ok('C1','no evaluation ever saw a config different from its entry admission config',mixed.length===0,{entries:all.length,distinctConfigs:hashes.size,mixed:mixed.length});
 C.ok('C1','v1 entries pending at the edit are Cancelled gate_definition_changed; none queued after it',v1Pending.length>0&&v1Pending.every(e=>e.failure==='gate_definition_changed')&&staleAfterEdit.length===0&&v1.every(e=>['queued','cancelled'].includes(e.disposition)),{v1:v1.length,v1Cancelled:v1Pending.length,v1Queued:v1.filter(e=>e.disposition==='queued').length,v1Other:v1.filter(e=>!['queued','cancelled'].includes(e.disposition)).map(e=>e.disposition+'/'+e.failure),staleAfterEdit:staleAfterEdit.length,editWindowMs:[ed.t0,ed.t1]});
 const labels=post.length?(await Promise.all(post.map(id=>rig.entry(A,id)))).map(e=>e.decisions.at(-1)?.report?.outcome?.Ok?.decision?.targets?.[0]?.payload?.label):[];
 C.ok('C1','entries admitted after the edit run with the new config (label v2, new hash)',labels.length===3&&labels.every(l=>l==='v2')&&v2.every(e=>e.config_sha256!==v1[0].config_sha256),{labels,v1hash:v1[0]?.config_sha256?.slice(0,19),v2hash:v2[0]?.config_sha256?.slice(0,19)});
 const dbDec=jq(rig,'select count(*) from workflow_gate_decisions')[0][0];const sumDec=all.reduce((a,e)=>a+e.decisions.length,0);
 C.ok('C1','journal decision rows == sum of entry decisions; handoffs consistent',dbDec===sumDec&&(S=await settled(rig)).ok,{dbDec,sumDec,S});}
// in-flight helper: hold a real evaluation with SIGSTOP, apply the disturbance, release, return observations
async function inflight(disturb){for(let attempt=1;attempt<=4;attempt++){const {entry:e0,worker:w}=await heldEval(rig,'run');const held=await rig.entry(A,e0.entry_id);
   if(held.disposition!=='evaluating'){sigWorker(w,'SIGCONT');await done(e0.entry_id,30000);continue}
   const v0=await rev();const d=await disturb();sigWorker(w,'SIGCONT');const e=await done(e0.entry_id,30000);await sleep(800);return {attempt,held:held.disposition,v0,v1:await rev(),e,d}}
  throw Error('could not hold an evaluation')}
// C2 edit while one evaluation is in flight
{const r=await inflight(()=>edit('v3'));
 C.ok('C2','config edit during in-flight evaluation: completion rejected (Cancelled gate_definition_changed), state not advanced, no handoff',r.held==='evaluating'&&r.e.disposition==='cancelled'&&r.e.failure==='gate_definition_changed'&&r.v1===r.v0&&r.e.queued_prompt_id===null&&(S=await settled(rig)).ok,{attempt:r.attempt,held:r.held,disp:r.e.disposition,failure:r.e.failure,revBefore:r.v0,revAfter:r.v1,S});}
// C3 reset-state during an in-flight evaluation
{const r=await inflight(()=>rig.reset(A,'wfgate reset during evaluation'));const st=await rig.state(A);
 C.ok('C3','reset-state during in-flight evaluation: entry Cancelled gate_state_reset, no state resurrected, no handoff',r.held==='evaluating'&&r.e.disposition==='cancelled'&&r.e.failure==='gate_state_reset'&&st.revision===0&&Object.keys(st.state.workflow).length===0&&r.e.queued_prompt_id===null&&(S=await settled(rig)).ok,{attempt:r.attempt,held:r.held,disp:r.e.disposition,failure:r.e.failure,revBefore:r.v0,snapshot:st,S});}
// C4 gate removed from the definition while entries are pending
{const ids=[];for(let i=0;i<4;i++)ids.push((await submit(rig,'defer:40')).entry_id);await sleep(1500);
 let applyRes;try{applyRes=Object.keys((await editConfig(rig,{config:null})).r)}catch(err){applyRes='refused: '+String(err).slice(0,200)}
 const all=await Promise.all(ids.map(id=>done(id,30000)));
 C.ok('C4','gate removed while pending: pending entries Cancelled gate_definition_changed, none queued',all.every(e=>e.disposition==='cancelled'&&e.failure==='gate_definition_changed'),{applyRes,all:all.map(e=>e.disposition+'/'+e.failure)});
 let ordinary;try{ordinary=Object.keys(await rig.invoke(A,'entry','plain'))}catch(err){ordinary='err '+String(err).slice(0,120)}
 C.ok('C4','after gate removal the endpoint runs ordinarily (no gate entry created)',Array.isArray(ordinary)&&!ordinary.includes('WorkflowEntryAccepted'),{ordinary});}
const sum=journalSummary(rig);rec('journal-summary',{result:'INFO',sum});
await rig.stop();console.log(JSON.stringify({failed:C.failed.length,total:C.rows.length}));
