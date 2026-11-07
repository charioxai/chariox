process.on("uncaughtException",(e)=>{console.error("UNCAUGHT",String(e).slice(0,160),(e.stack||"").split("\n").filter(l=>l.includes("wfgate-r1")).slice(0,3).join(" | "));process.exit(2)});
import {Rig,boot,submit,sleep,R,recorder,jq,runs,kinds,isTerminal,Checks,journalSummary,settled,handoffSets,sha} from './lib.mjs';
import {readFileSync} from 'node:fs';
const rec=recorder('g07');const C=new Checks(rec);let S;
const rig=new Rig('g07a');await rig.start();await rig.session();await boot(rig);
const A=rig.alias;const done=(id,ms=120000)=>rig.waitFor(A,id,isTerminal,ms,'terminal');
const rev=async()=>(await rig.state(A));
const waitAll=async(ids,ms=150000)=>{const t=Date.now();for(;;){const all=await rig.allEntries(A);const m=new Map(all.map(e=>[e.entry_id,e]));const sel=ids.map(i=>m.get(i));if(sel.every(e=>e&&isTerminal(e)))return Promise.all(ids.map(i=>rig.entry(A,i)));if(Date.now()-t>ms)throw Error('waitAll timeout '+sel.filter(e=>!e||!isTerminal(e)).length);await sleep(700)}};
let overloaded=0;
const sub=async(p)=>{for(let i=0;i<60;i++){try{return await submit(rig,p)}catch(e){if(String(e).includes('overloaded')){overloaded++;await sleep(150+Math.random()*300);continue}throw e}}throw Error('still overloaded')};
const consistent=async(name)=>{const all=await rig.allEntries(A);const dec=jq(rig,'select count(*) from workflow_gate_decisions')[0][0];const sumDec=all.reduce((a,e)=>a+e.evaluation_count,0);
  const ok=jq(rig,"select count(*) from workflow_gate_decisions where body_json like '%\"Ok\"%'")[0][0];const st=await rev();const stuck=all.filter(e=>!isTerminal(e)).length;
  return {entries:all.length,dec,sumDec,okEvals:ok,stuck,stateEvaluated:st.state.workflow.evaluated,stateRevision:st.revision}};
// O2 50 parallel invokes
{const prompts=[...Array(30).fill('run'),...Array(10).fill('drop'),...Array(10).fill('defer:3')];
 const t=Date.now();const res=await Promise.all(prompts.map(p=>sub(p)));const ids=res.map(e=>e.entry_id);
 const fin=await waitAll(ids);const secs=(Date.now()-t)/1000;const c=await consistent();
 const dd=jq(rig,"select count(distinct dedupe_id),count(*) from workflow_gate_entries")[0];
 C.ok('O2','50 parallel invokes: 50 distinct entries, all terminal',new Set(ids).size===50&&fin.every(isTerminal)&&c.entries===50&&dd[0]===50,{secs,overloadRetries:rig.overloaded||0,c,dedupe:dd,dispositions:Object.fromEntries(Object.entries(fin.reduce((a,e)=>(a[e.disposition]=(a[e.disposition]||0)+1,a),{})))});
 C.ok('O2','journal decision rows == evaluation counts; state revision == successful evaluations == state counter; handoffs exact',c.dec===c.sumDec&&c.stateRevision===c.okEvals&&c.stateEvaluated===c.stateRevision&&c.stuck===0&&(S=await settled(rig)).ok,{c,S});}
// O3 storm of control operations against live evaluations
{const pend=[];for(let i=0;i<20;i++)pend.push((await submit(rig,'defer:12')).entry_id);
 const victim=pend[5];const wasm=readFileSync(R+'/matrix-gate.wasm');
 const ops=[];const tag=(n,p)=>ops.push(p.then(v=>({n,ok:true,v}),e=>({n,ok:false,e:String(e).slice(0,160)})));
 for(let i=0;i<6;i++)tag('reset',rig.reset(A,'storm reset '+i));
 for(let i=0;i<8;i++)tag('put',rig.gate('gate-upload',{kind:'put_module',wasm_base64:wasm.toString('base64'),sha256:sha(wasm)}));
 for(let i=0;i<8;i++)tag('cancel',rig.cancel(A,victim,'storm cancel '+i));
 for(let i=0;i<4;i++)tag('cancel-bogus',rig.cancel(A,'gate_doesnotexist'+i,'x'));
 const late=[];for(let i=0;i<10;i++)tag('invoke',sub('run').then(e=>{late.push(e.entry_id);return e}));
 const out=await Promise.all(ops);
 const sum={};for(const o of out){const k=o.n+':'+(o.ok?'ok':'err');sum[k]=(sum[k]||0)+1}
 const errs=out.filter(o=>!o.ok).map(o=>o.n+' '+o.e).slice(0,6);
 const all=[...pend,...late];const fin=await waitAll(all);const c=await consistent();
 const allowedErr=/already handed or terminal|not found|unknown|no such/i;C.ok('O3','storm: all 36 operations return a result; only defined refusals (cancel of an entry already cancelled by a concurrent reset, bogus ids)',out.length===36&&out.filter(o=>o.n!=='cancel'&&o.n!=='cancel-bogus').every(o=>o.ok)&&out.filter(o=>!o.ok).every(o=>allowedErr.test(o.e))&&out.filter(o=>o.n==='cancel-bogus').every(o=>!o.ok),{sum,errs:[...new Set(out.filter(o=>!o.ok).map(o=>o.e.replace(/.*failed: /,'')))]});
 C.ok('O3','after the storm: all terminal; state counter == revision; decision rows == evaluations except claims cancelled in flight by a reset; handoffs exact',fin.every(isTerminal)&&c.stuck===0&&c.dec<=c.sumDec&&c.sumDec-c.dec<=fin.filter(e=>e.failure==='gate_state_reset').length&&c.stateEvaluated===c.stateRevision&&(S=await settled(rig)).ok,{c,S,dispositions:fin.reduce((a,e)=>(a[e.disposition+(e.failure?'/'+e.failure:'')]=(a[e.disposition+(e.failure?'/'+e.failure:'')]||0)+1,a),{})});}
// O4 export / transfer / owner scope: second kernel
{const exp=(await rig.send({ExportWorkflowCodeSource:{session_id:rig.sessionId,target:{kind:'workflow',workflow_ref:A},format:'inline',agent_mode:'portable_generated'}})).WorkflowCodeSourceExported.export;
 const e1=await done((await submit(rig,'run')).entry_id);
 const B=new Rig('g07b');await B.start();await B.session();await B.newWorkflow('gate-upload');
 let refusal=null,applied=null;try{applied=await B.apply(exp.source)}catch(err){refusal=String(err).slice(0,220)}
 let ranWithoutModule=null;if(applied){try{const e=await B.invoke('gw','entry','run');ranWithoutModule=JSON.stringify(e).slice(0,160)}catch(err){ranWithoutModule='refused: '+String(err).slice(0,160)}}
 C.ok('O4','transfer without the module: no evaluation, no run on the destination owner',(refusal!==null||(applied&&String(ranWithoutModule).startsWith('refused')))&&jq(B,'select count(*) from workflow_gate_decisions')[0][0]===0,{refusal,appliedKeys:applied?Object.keys(applied):null,ranWithoutModule});
 let after=null;try{const m=await B.put('gate-upload',R+'/matrix-gate.wasm');if(!applied)applied=await B.apply(exp.source);B.alias='gw';const e=await B.invoke('gw','entry','run');const id=(e.WorkflowEntryAccepted??{}).entry?.entry_id;const f=await B.waitFor('gw',id,isTerminal,60000);after={module:m,disp:f.disposition,module_sha256:f.module_sha256,config_sha256:f.config_sha256}}catch(err){after='ERR '+String(err).slice(0,300)}
 C.ok('O4','after uploading the portable module for the destination owner: same digests, entry evaluates and queues',typeof after==='object'&&after.disp==='queued'&&after.module_sha256===e1.module_sha256&&after.config_sha256===e1.config_sha256,{after,source:{module:e1.module_sha256,config:e1.config_sha256}});
 await B.stop();}
const sum=journalSummary(rig);rec('journal-summary',{result:'INFO',sum});
await rig.stop();console.log(JSON.stringify({failed:C.failed.length,total:C.rows.length}));
