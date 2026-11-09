// wfreal control: a real Codex profile on a throwaway kernel. Steps (argv): plain | direct | roomtool | f2real
import {Rig,sleep,runs,wfSource,recorder} from './lib.mjs';
import {execFileSync} from 'node:child_process';
const TREE=new URL('../../',import.meta.url).pathname.replace(/\/$/,'');
const requests=await import(TREE+'/packages/kernel-client/dist/ipc-requests.js');
const step=process.argv[2]||'plain';const rec=recorder('wfreal-'+step);
const T0=Date.now();const ts=()=>((Date.now()-T0)/1000).toFixed(1).padStart(6)+'s';
const say=(...a)=>console.log(ts(),...a);
const CODEX_SRC='/root/.chariox/dev/worker-k/home/state/provider-accounts/local/codex/kaissandra-ai-reviewer-login-esqv7b7c/codex';
const rig=new Rig('wfreal-'+step,{extraEnv:{CHARIOX_CODEX_BIN:'/usr/local/bin/codex',IS_SANDBOX:'1',CHARIOX_ROOM_AGENT_TOOLS:'1',...(process.env.WFREAL_ENV?JSON.parse(process.env.WFREAL_ENV):{})}});
(await import('node:fs')).rmSync(rig.dir,{recursive:true,force:true});await rig.start();
const link=await rig.send({LinkProviderAccountProfile:{provider:'codex',label:'wfreal-luna',path:CODEX_SRC}});
const prof=link.ProviderAccountProfile.profile.profile_id;say('profile linked',!!prof);
if(!process.env.WFREAL_NOREFRESH){const rf=await rig.send({RefreshProviderAccountProfile:{provider:'codex',account_profile:prof}});say('refresh',JSON.stringify(rf).replace(/"[^"]*(token|secret|key)[^"]*":"[^"]*"/gi,'').slice(0,200))}
const ws=`${rig.dir}/ws`;(await import('node:fs')).mkdirSync(ws,{recursive:true});
const made=(await rig.send({CreateSession:{workspace_id:ws,worktree_id:ws,alias:'room1',agent_defaults:{provider:'codex',account_profile:prof,model:'gpt-6-luna',effort:'low',permission_level:'yolo'}}})).SessionCreated;
rig.sessionId=made.session.id;const roomAgent=made.agent.id;
const att=(await rig.send({AttachToSession:{session_id:rig.sessionId,client_id:'wfreal',capability_level:'FullTerminal'}})).SessionAttached.attachment.id;
say('room',rig.sessionId,'agent',roomAgent);
const snap=r=>({id:r.id?.slice(0,8),status:r.status,nodes:(r.node_runs??[]).map(n=>n.node_id+':'+n.status+'/'+(n.turn_envelope?.state??'-')+(n.summary?' "'+String(n.summary).slice(0,60)+'"':''))});
let first=true;
async function watch(ms,done){const t=Date.now();let last='';let rfd=false;for(;;){if(process.env.WFREAL_REFRESH_AT&&!rfd&&Date.now()-t>+process.env.WFREAL_REFRESH_AT){rfd=true;await rig.send({RefreshProviderAccountProfile:{provider:'codex',account_profile:prof}});say('LATE REFRESH sent')}const rs=await runs(rig);if(first&&rs[0]){first=false;say('RUN KEYS',Object.keys(rs[0]).join(','))}const s=JSON.stringify(rs.map(snap));if(s!==last){say('runs',s);last=s}if(done(rs))return rs;if(Date.now()-t>ms)return rs;await sleep(500)}}
async function agentState(){return (await rig.send({GetSessionState:{session_id:rig.sessionId}})).SessionState.session}
if(step==='plain'){
  const r=await rig.send(requests.submitPromptRequest(rig.sessionId,att,roomAgent,'Reply with exactly: OK',[]));say('submitted');
  for(let i=0;i<60;i++){await sleep(2000);const s=await agentState();const a=s.active_prompt;say('active_prompt',a?a.status:'none');if(!a&&i>1)break}
}
if(step==='direct'||step==='f2real'){
  await rig.newWorkflow('ctl');
  const gated=step==='f2real';let module=null,config=null;
  if(gated){await rig.newWorkflow('gate-upload');module=await rig.put('gate-upload',rig.dir+'/../matrix-gate.wasm');config={label:'v1',step_ms:300}}
  const src=wfSource({alias:'gw',module,config,provider:'codex',model:'gpt-6-luna',agentExtra:{effort:'low',accountProfile:prof}});
  const rep=await rig.apply(src);say('applied',JSON.stringify(rep).slice(0,160));rig.alias='gw';rig.module=module;rig.config=config;rig.nodes=['entry'];
  const e=await rig.invoke('gw','entry','run');say('invoked',JSON.stringify(e).slice(0,200));
  if(process.env.CANCEL_RUN==='1'){const codexKids=()=>rig.kids().filter(k=>/codex/i.test(k.comm)).map(k=>k.pid+':'+k.comm);let rs0;for(let i=0;i<120;i++){rs0=await runs(rig);if(rs0[0]?.node_runs?.some(n=>n.status==='Running'))break;await sleep(200)}
    say('TREE before cancel',JSON.stringify((()=>{const ids=rig.kids().map(k=>String(k.pid));return ids.length?execFileSync('ps',['-o','pid=,ppid=,etimes=,stat=,args=','-p',ids.join(',')],{encoding:'utf8'}).trim().split('\n').map(l=>l.trim().replace(/\s+/g,' ').slice(0,160)):[]})()));say('CANCEL_RUN at',JSON.stringify(rs0.map(snap)),'codex',JSON.stringify(codexKids()));const tc=Date.now();let ans='pending';rig.send({CancelWorkflowRun:{session_id:rig.sessionId,workflow_run_ref:rs0[0].id}}).then(r=>{ans='ANSWER';say('cancel run answered after',Date.now()-tc,'ms',JSON.stringify(r).slice(0,160))},e=>{ans='ERR';say('cancel run error after',Date.now()-tc,'ms',String(e).slice(0,200))});
    const OBS=+(process.env.OBSERVE_MS||0);const tree=()=>{const ids=rig.kids().map(k=>String(k.pid));return ids.length?execFileSync('ps',['-o','pid=,ppid=,etimes=,stat=,args=','-p',ids.join(',')],{encoding:'utf8'}).trim().split('\n').map(l=>l.trim().replace(/\s+/g,' ').slice(0,160)):[]};const marks=[5000,30000,90000];
    for(let i=0;i<(OBS?OBS/500:120);i++){await sleep(500);const rs1=await runs(rig);if(i%4===0)say('after cancel',JSON.stringify(rs1.map(snap)),'codex',JSON.stringify(codexKids()),ans);while(marks.length&&Date.now()-tc>=marks[0])say('TREE +'+marks.shift()/1000+'s',JSON.stringify(tree()));if(!OBS&&ans!=='pending'&&i>4&&codexKids().length<=1)break}
    for(let r=2;r<=(+process.env.REPEAT||1);r++){const e2=await rig.invoke('gw','entry','run');say('invoked again',r,JSON.stringify(e2).slice(0,80));let rs2;for(let i=0;i<120;i++){rs2=await runs(rig);if(rs2.some(x=>x.status==='Running'&&x.node_runs?.some(n=>n.status==='Running')))break;await sleep(200)}
      const live=rs2.find(x=>x.status==='Running');say('TREE before cancel',r,JSON.stringify(tree()));await rig.send({CancelWorkflowRun:{session_id:rig.sessionId,workflow_run_ref:live.id}});say('cancelled',r);await sleep(+process.env.REPEAT_OBS_MS||20000);say('TREE after cancel',r,JSON.stringify(tree()),'runs',JSON.stringify((await runs(rig)).map(snap)))}
    rec('cancel-run-real-codex',{result:ans==='ANSWER'?'INFO-answered':'INFO-pending',afterMs:Date.now()-tc});await rig.stop();process.exit(0)}
  const rs=await watch(+(process.env.WAIT||150000),rs=>rs.length>=1&&rs.every(r=>['Completed','Failed','Stopped'].includes(r.status)));
  rec('final',{result:'INFO',runs:rs.map(snap)});
  if(gated){const es=await rig.allEntries('gw');say('ENTRIES',JSON.stringify(es.map(x=>({d:x.disposition,f:x.failure,q:x.queued_prompt_id?.slice?.(0,8)}))));const q=await rig.send({ListQueuedWorkflowPrompts:{session_id:rig.sessionId}});say('QUEUED',JSON.stringify(q).slice(0,300))}
  if(rs[0])say('RUN0',JSON.stringify(rs[0]).slice(0,900));
}
if(process.env.KEEP!=='1'){await rig.stop();say('stopped')}
