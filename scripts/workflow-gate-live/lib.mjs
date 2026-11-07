// wfgate r1 driver: own kernel on own CHARIOX_HOME, raw local IPC requests.
import {spawn,execFileSync} from 'node:child_process';
import {createHash,randomBytes} from 'node:crypto';
import {readFileSync,writeFileSync,mkdirSync,openSync,existsSync,appendFileSync} from 'node:fs';
import net from 'node:net';
const TREE='/root/work/agent-wfgate';
const {LocalIpcClient}=await import(TREE+'/packages/kernel-client/dist/ipc.js');
export const R='/root/.chariox/dev/wfgate-r1';
export const EV='/root/.codex/evidence/wfgate/r1';
export const sleep=ms=>new Promise(r=>setTimeout(r,ms));
export const sha=b=>'sha256:'+createHash('sha256').update(b).digest('hex');
export const unwrap=(r,k)=>{if(!r||!r[k])throw Error('expected '+k+' got '+JSON.stringify(r).slice(0,400));return r[k]};
async function freePort(){return new Promise((res,rej)=>{const s=net.createServer();s.on('error',rej);s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>res(p))})})}
const LIVE=new Set();
for(const ev of ['exit','SIGINT','SIGTERM'])process.on(ev,()=>{for(const r of LIVE){try{if(r.pid>1&&!r.exited)process.kill(r.pid,'SIGKILL')}catch{}}if(ev!=='exit')process.exit(1)});
export class Rig{
  constructor(name,{extraEnv={}}={}){this.name=name;this.dir=`${R}/run-${name}`;this.extraEnv=extraEnv;this.steps=[]}
  async start(){
    const d=this.dir;for(const s of ['home','tmp','os','cfg','data','state','cache'])mkdirSync(`${d}/${s}`,{recursive:true,mode:0o700});
    if(!this.ports)this.ports=await Promise.all([1,2,3,4].map(freePort));
    const p=this.ports;
    this.env={PATH:process.env.PATH,LANG:'C.UTF-8',HOME:`${d}/os`,CHARIOX_HOME:`${d}/home`,TMPDIR:`${d}/tmp`,XDG_CONFIG_HOME:`${d}/cfg`,XDG_DATA_HOME:`${d}/data`,XDG_STATE_HOME:`${d}/state`,XDG_CACHE_HOME:`${d}/cache`,CHARIOX_KERNEL_PORT:String(p[0]),CHARIOX_MCP_PORT:String(p[1]),CHARIOX_CODEX_PORT:String(p[2]),CHARIOX_OPENCODE_PORT:String(p[3]),CHARIOX_DAEMON_SOCKET:`${d}/daemon.sock`,RUST_BACKTRACE:'0',...this.extraEnv};
    const out=openSync(`${d}/kernel.log`,'a',0o600);
    const wrap=process.env.WF_STRACE?['strace',['-f','-qq','-o',process.env.WF_STRACE,'-e','trace=execveat,openat,prctl,landlock_create_ruleset,landlock_add_rule,landlock_restrict_self,setrlimit,prlimit64,close_range,personality,seccomp,execve,unshare,clone3',`${R}/bin/chariox-kernel`]]:[`${R}/bin/chariox-kernel`,[]];
    this.child=spawn(wrap[0],wrap[1],{cwd:`${R}/ws`,env:this.env,stdio:['ignore',out,out],detached:true});
    this.pid=this.child.pid;LIVE.add(this);writeFileSync(`${d}/kernel.pid`,String(this.pid));
    this.child.on('exit',(c,s)=>{this.exited={c,s}});
    await this.connect();return this}
  async connect(){for(let i=0;i<240;i++){if(this.exited)throw Error('kernel exited '+JSON.stringify(this.exited));const c=new LocalIpcClient(`ws://127.0.0.1:${this.ports[0]}/kernel`,{timeoutMs:20000,localAuthEnvironment:this.env});try{await c.send({ListSessions:null});this.client=c;return}catch(e){this.lastErr=String(e).slice(0,200);try{await c.close()}catch{}await sleep(500)}}throw Error('kernel not ready '+this.lastErr)}
  async send(req){for(let i=0;;i++){try{return await this.client.send(req)}catch(e){if(String(e).includes('kernel_request_overloaded')||String(e).includes('admission queue overloaded')){this.overloaded=(this.overloaded||0)+1;if(i>80)throw e;await sleep(100+Math.random()*250);continue}throw e}}}
  async stop(sig='SIGTERM'){if(!this.pid||this.exited)return;try{await this.client.close()}catch{}
    if(!(this.pid>1))throw Error('bad pid');process.kill(this.pid,sig);for(let i=0;i<120&&!this.exited;i++)await sleep(250);if(!this.exited){process.kill(this.pid,'SIGKILL');for(let i=0;i<40&&!this.exited;i++)await sleep(250)}}
  async kill9(){if(!(this.pid>1))throw Error('bad pid');process.kill(this.pid,'SIGKILL');for(let i=0;i<80&&!this.exited;i++)await sleep(100);try{this.client.destroy()}catch{}}
  async restart(extra){this.exited=undefined;if(extra)Object.assign(this.extraEnv,extra);await this.start();if(this.sessionId)await this.attach?.()}
  // descendants (exact pids) of the kernel
  kids(){try{const out=execFileSync('ps',['-eo','pid=,ppid=,stat=,comm='],{encoding:'utf8'}).trim().split('\n').map(l=>l.trim().split(/\s+/));const by=new Map();for(const [pid,ppid,stat,comm] of out){by.set(+pid,{pid:+pid,ppid:+ppid,stat,comm})}
    const res=[];const walk=(pp)=>{for(const v of by.values())if(v.ppid===pp){res.push(v);walk(v.pid)}};walk(this.pid);return res}catch{return []}}
  async session(){const r=await this.send({CreateSession:{workspace_id:`${R}/ws`,worktree_id:`${R}/ws`,alias:null,slice_ref:null}});const s=unwrap(r,'SessionCreated').session;this.sessionId=s.id;return s.id}
  // gate requests
  gate(workflow,action){return this.send({WorkflowGate:{session_id:this.sessionId,workflow_ref:workflow,action}})}
  async put(workflow,wasmPath){const b=readFileSync(wasmPath);return unwrap(await this.gate(workflow,{kind:'put_module',wasm_base64:b.toString('base64'),sha256:sha(b)}),'WorkflowGateModuleStored').sha256}
  async entries(workflow,after=null,limit=32){return unwrap(await this.gate(workflow,{kind:'entries',after,limit}),'WorkflowGateEntries')}
  async allEntries(workflow){let after=null,all=[];for(;;){const p=await this.entries(workflow,after);all.push(...p.entries);if(!p.next_cursor)break;after=p.next_cursor}return all}
  async entry(workflow,id){const r=await this.gate(workflow,{kind:'entry',entry_id:id});return r.WorkflowEntryAccepted?.entry??r}
  async state(workflow,binding=null){return unwrap(await this.gate(workflow,{kind:'state',binding_id:binding}),'WorkflowGateState').snapshot}
  async reset(workflow,reason){return unwrap(await this.gate(workflow,{kind:'reset_state',reason}),'WorkflowGateState').snapshot}
  async cancel(workflow,id,reason){return this.gate(workflow,{kind:'cancel',entry_id:id,reason})}
  async newWorkflow(alias){return this.send({CreateWorkflow:{session_id:this.sessionId,alias}})}
  async apply(source){return this.send({ApplyWorkflowCode:{session_id:this.sessionId,node_path:process.execPath,source}})}
  async invoke(workflow,endpoint,prompt,queue=null){return this.send({InvokeWorkflowEndpoint:{session_id:this.sessionId,workflow_ref:workflow,endpoint_ref:endpoint,queue_ref:queue,prompt}})}
  async waitFor(workflow,id,pred,ms=60000,label=''){const t=Date.now();let e;for(;;){e=await this.entry(workflow,id);if(pred(e))return e;if(Date.now()-t>ms)throw Error(`timeout ${label} ${id} last=${e.disposition}/${e.failure}`);await sleep(200)}}
}
export const canon=o=>JSON.stringify(sortKeys(o));
function sortKeys(v){if(Array.isArray(v))return v.map(sortKeys);if(v&&typeof v==='object')return Object.fromEntries(Object.keys(v).sort().map(k=>[k,sortKeys(v[k])]));return v}
export function wfSource({alias,module,config,provider='dev-stub',model='default',nodes=['entry'],maxConcurrent=32,agentExtra={}}){
  const cfg=config?JSON.parse(canon(config)):null;
  const def={alias,maxConcurrent,flushAgentContextBeforeRun:false};if(cfg)def.gate={abi:'gate_v1',module_sha256:module,config:cfg,config_sha256:sha(canon(cfg))};
  let s=`workflow.define(${JSON.stringify(def)})\n`;
  const hs=nodes.map(n=>`n_${n}`);
  for(const n of nodes)s+=`const n_${n}=workflow.node({handle:${JSON.stringify(n)},agent:workflow.newAgent({alias:${JSON.stringify(alias+'-'+n)},provider:${JSON.stringify(provider)},model:${JSON.stringify(model)}${Object.entries(agentExtra).map(([k,v])=>','+k+':'+JSON.stringify(v)).join('')}}),instructions:'Acknowledge the handoff with one word.',canCompleteWorkflowRun:true,maxTurns:1})\n`;
  s+=cfg?`workflow.endpoint([${hs.join(',')}],{alias:'entry'})\n`:`workflow.endpoint(${hs[0]},{alias:'entry'})\n`;
  return s}
const log=(f,o)=>appendFileSync(f,JSON.stringify({t:new Date().toISOString(),...o})+'\n');
export function recorder(file){mkdirSync(EV,{recursive:true});return (name,o)=>{log(`${EV}/${file}.jsonl`,{name,...o});console.log(file+': '+name+' '+(o.result??''))}}
export async function boot(rig,{alias='gw',config={label:'v1',step_ms:300},nodes=['entry'],wasm=R+'/matrix-gate.wasm',provider='dev-stub',model='default',agentExtra={}}={}){
  await rig.newWorkflow('gate-upload');
  const module=await rig.put('gate-upload',wasm);
  const rep=await rig.apply(wfSource({alias,module,config,nodes,provider,model,agentExtra}));
  rig.alias=alias;rig.module=module;rig.config=config;rig.nodes=nodes;return {module,rep}}
export async function submit(rig,prompt,alias=rig.alias){const r=await rig.invoke(alias,'entry',prompt);
  const e=r.WorkflowEntryAccepted?.entry??r.WorkflowRunInvoked?.entry??r.entry;if(!e?.entry_id)throw Error('no entry in '+JSON.stringify(r).slice(0,500));return e}
export function jq(rig,sql,args=[]){const code=`import sqlite3,json,sys\nc=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True,timeout=10)\nprint(json.dumps(c.execute(sys.argv[2],json.loads(sys.argv[3])).fetchall()))`;
  return JSON.parse(execFileSync('python3',['-I','-c',code,`${rig.dir}/home/state/kernel.db`,sql,JSON.stringify(args)],{encoding:'utf8'}))}
export const TERMINAL=['queued','dropped','failed','expired','cancelled'];
export const isTerminal=e=>TERMINAL.includes(e.disposition);
export async function runs(rig,alias=rig.alias){const r=await rig.send({ListWorkflowRuns:{session_id:rig.sessionId,workflow_ref:alias}});return r.WorkflowRunsListed.workflow_runs}
export const kinds=e=>e.decisions.map(d=>d.report?.outcome?.Ok?.decision?.kind??(d.report?.outcome?.Err?'ERR:'+d.report.outcome.Err:(d.compacted?'compacted':'?')));
export class Checks{constructor(rec){this.rows=[];this.rec=rec}
  ok(id,name,pass,detail){const row={id,name,result:pass?'PASS':'FAIL',detail};this.rows.push(row);this.rec(id+' '+name,{result:row.result,detail});return pass}
  get failed(){return this.rows.filter(r=>r.result!=='PASS')}}
import {readdirSync} from 'node:fs';
export function findWorker(rig){try{for(const t of readdirSync(`/proc/${rig.pid}/task`)){const c=readFileSync(`/proc/${rig.pid}/task/${t}/children`,'utf8').trim();for(const p of c.split(/\s+/).filter(Boolean)){try{if(readFileSync(`/proc/${p}/cmdline`,'utf8').includes('gate-supervised'))return +p}catch{}}}}catch{}
  try{const out=execFileSync('pgrep',['-f','gate-supervised','-P',String(rig.pid)],{encoding:'utf8',stdio:['ignore','pipe','ignore']}).trim().split('\n').filter(Boolean);if(out.length)return +out[0]}catch{}return null}
export async function catchWorker(rig,ms=15000){const t=Date.now();for(;;){const p=findWorker(rig);if(p)return p;if(Date.now()-t>ms)throw Error('no worker seen');await sleep(2)}}
export function sigWorker(pid,signal){if(!(pid>1))throw Error('bad pid');let cl='';try{cl=readFileSync(`/proc/${pid}/cmdline`,'utf8')}catch{return false}if(!cl.includes('gate-supervised'))throw Error('refusing to signal non-worker');process.kill(pid,signal);return true}
export function journalSummary(rig){return {entries:jq(rig,"select disposition,count(*) from workflow_gate_entries group by disposition"),decisions:jq(rig,"select count(*) from workflow_gate_decisions")[0][0],state:jq(rig,"select binding_id,revision,state_json from workflow_gate_state")}}
export async function handoffs(rig,alias=rig.alias){const r=await runs(rig,alias);const q=await rig.send({ListQueuedWorkflowPrompts:{session_id:rig.sessionId}});const qp=(q.QueuedWorkflowPromptsListed?.queued_prompts??[]);return r.length+qp.length}

// Every Queued gate entry must own exactly one handoff: a durable queued prompt or a run's invocation message. Never extra, never missing.
export async function settled(rig,ms=20000){const t=Date.now();let r;for(;;){r=await handoffSets(rig);if(r.ok||Date.now()-t>ms)break;await sleep(500)}await sleep(800);return await handoffSets(rig)}
export async function handoffSets(rig){const queuedIds=jq(rig,"select case when json_valid(body_json) then json_extract(body_json,'$.entry.queued_prompt_id') else json_extract(summary_json,'$.queued_prompt_id') end from workflow_gate_entries where disposition='queued'").map(r=>r[0]);
  const distinct=new Set(queuedIds).size;
  const durable=jq(rig,"select count(*) from durable_workflow_hot_entities where entity_kind='queued_prompt'")[0][0];
  const rs=await runs(rig);const inRuns=rs.length;
  // batch members share their aggregate's single prompt id; every distinct prompt id owns exactly one queued prompt or one run
  return {queuedEntries:queuedIds.length,distinctPromptIds:distinct,durableQueuedPrompts:durable,runs:inRuns,ok:durable+inRuns===distinct}}
export async function heldEval(rig,prompt,signal='SIGSTOP',tries=5){for(let i=0;i<tries;i++){const e=await submit(rig,prompt);try{const w=await catchWorker(rig,4000);sigWorker(w,signal);return {entry:e,worker:w}}catch(err){await rig.waitFor(rig.alias,e.entry_id,isTerminal,30000)}}throw Error('could not hold an evaluation')}
export async function wfRevision(rig,alias=rig.alias){const r=await rig.send({ListWorkflows:{session_id:rig.sessionId}});const l=r.WorkflowsListed?.workflows??r.WorkflowListed?.workflows??Object.values(r)[0]?.workflows??[];const w=l.find(x=>x.alias===alias||x.id===alias);if(!w)throw Error('workflow not listed: '+JSON.stringify(r).slice(0,300));return {revision:w.revision??w.workflow_revision,id:w.id,w}}
// In-place config edit the product way: authored code artifact, updated, bound and rebuilt over the same workflow.
export async function editConfig(rig,{config}){const t0=Date.now();const name='src-'+rig.alias;const lang='java_script';const mk=c=>wfSource({alias:rig.alias,module:rig.module,config:c,nodes:rig.nodes});
  const call=r=>rig.send(r);
  if(!rig.bound){await call({CreateWorkflowCodeArtifact:{session_id:rig.sessionId,name,language:lang,node_path:process.execPath,source:mk(rig.config)}})}
  else await call({UpdateWorkflowCodeArtifact:{session_id:rig.sessionId,name,language:lang,node_path:process.execPath,source:mk(config)}});
  let {revision}=await wfRevision(rig);
  await call({BindWorkflowCodeSource:{session_id:rig.sessionId,workflow_ref:rig.alias,artifact_name:name,origin:'authored',expected_workflow_revision:revision}});
  if(!rig.bound){rig.bound=true;await call({UpdateWorkflowCodeArtifact:{session_id:rig.sessionId,name,language:lang,node_path:process.execPath,source:mk(config)}});({revision}=await wfRevision(rig));await call({BindWorkflowCodeSource:{session_id:rig.sessionId,workflow_ref:rig.alias,artifact_name:name,origin:'authored',expected_workflow_revision:revision}})}
  ({revision}=await wfRevision(rig));
  const r=await call({RebuildWorkflowCodeSource:{session_id:rig.sessionId,workflow_ref:rig.alias,expected_workflow_revision:revision,confirm:true}});rig.config=config;return {t0,t1:Date.now(),r,revisionBefore:revision}}
