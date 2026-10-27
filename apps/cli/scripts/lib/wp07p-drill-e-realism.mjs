// MP-08/MP-10: official-provider smoke, separate from ctlconc timing acceptance.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import {readDrillEActionHistory} from '../live-browser-computer-drill-e.mjs';
import {roomActionNoticePattern,automationNoticeTexts} from './room-tui-notices.mjs';
const mp_items=['MP-08','MP-10'];
export function isHumanRoomReload(action,environment,baseline){
 return action.sequence>baseline&&environment.actors.some(actor=>actor.actor_id===action.actor_id&&actor.kind==='human')&&action.kind==='browser_history_reload'&&action.state==='completed';
}
export async function runERealism(i){
 const {client,requests,sessionId,agentIds,sameTabId,otherTabId,turn,waitFor,out,localAutomation,remoteAutomation,web}=i;
 const result={mp_items,status:'RED',scope:'mixed official-provider realism smoke, not timing-exact acceptance',checks:{},promptIds:[],actions:[],doesNotEstablish:['physical read overlap','exact queue/cancellation timing','managed Path-1','MP closure']};
 const getEnv=async()=>(await client.send(requests.getRoomEnvironmentStateRequest(sessionId))).RoomEnvironmentState.environment;
 const base=await readDrillEActionHistory(client,requests,sessionId),baseline=Math.max(0,...base.map(a=>a.sequence));
 let owned=false;
 try{
  await Promise.all(agentIds.map(async(id,n)=>result.promptIds.push(await turn(id,n<2?'MP-08/MP-10 realism read: call slice_browser_status, then slice_browser_find with query "Drill E probe" and kind "any" on the focused tab, then stop.':`MP-08/MP-10 realism independent work: call slice_browser_history exactly once, tab_id "${otherTabId}", action "reload", then stop.`))));
  result.checks.threeAgentDispatch=true;
  await Promise.all(agentIds.map(async(id,n)=>result.promptIds.push(await turn(id,`MP-08/MP-10 realism mutation: call slice_browser_history exactly once, tab_id "${n<2?sameTabId:otherTabId}", action "reload", then stop.`))));
  let env=await getEnv();assert.equal(env.focused_tab_id,sameTabId,'MP-08/MP-10 agent action changed selected tab');
  await client.send(requests.requestRoomEnvironmentInputTakeoverRequest(sessionId,{kind:'browser_tab',id:sameTabId}));
  env=await waitFor(async()=>{const e=await getEnv();const actors=new Map(e.actors.map(a=>[a.actor_id,a.kind]));return e.input_ownership.some(x=>x.target.kind==='browser_tab'&&x.target.id===sameTabId&&actors.get(x.actor_id)==='human')?e:false},15000,'MP-08/MP-10 realism human ownership absent');owned=true;
  await client.send(requests.submitRoomEnvironmentBrowserActionRequest(sessionId,env.runtime_generation,randomUUID(),{kind:'history',tab_id:sameTabId,action:'reload'}));
  await waitFor(async()=>{const a=await readDrillEActionHistory(client,requests,sessionId);return a.find(x=>isHumanRoomReload(x,env,baseline))},15000,'MP-08/MP-10 human reload missing');
  result.checks.kernelHumanTakeover=true;
  await client.send(requests.releaseRoomEnvironmentInputRequest(sessionId,{kind:'browser_tab',id:sameTabId}));owned=false;
  const actions=(await readDrillEActionHistory(client,requests,sessionId)).filter(a=>a.sequence>baseline);
  result.actions=actions.map(a=>({actionId:a.action_id,sequence:a.sequence,actorId:a.actor_id,kind:a.kind,state:a.state,outcome:a.outcome,targets:a.targets,submittedAt:a.submitted_at_ms,startedAt:a.started_at_ms,finishedAt:a.finished_at_ms}));
  for(const [n,id] of agentIds.entries()){
   assert.ok(actions.some(a=>a.actor_id==='agent:'+id&&a.state==='completed'&&(n<2?['browser_status','browser_find'].includes(a.kind):a.kind==='browser_history_reload')),'MP-08/MP-10 native agent read/work missing');
   assert.ok(actions.some(a=>a.actor_id==='agent:'+id&&a.kind==='browser_history_reload'&&a.state==='completed'&&a.targets.some(t=>t.kind==='browser_tab'&&t.id===(n<2?sameTabId:otherTabId))),'MP-08/MP-10 native mutation target missing');
  }
  result.checks.nativeReadsAndTargetedMutations=true;
  const noticeActions=actions.filter(a=>a.state==='completed'&&a.kind==='browser_history_reload');
  for(const [name,tui] of [['localTui',localAutomation],['remoteTui',remoteAutomation]]){
   await waitFor(async()=>{const texts=automationNoticeTexts(await tui.send('snapshot'));return noticeActions.every(a=>texts.some(text=>roomActionNoticePattern(a).test(text)))},15000,'MP-08/MP-10 realism TUI action attribution missing');result.checks[name]=true;
  }
  env=await getEnv();assert.equal(env.focused_tab_id,sameTabId);result.checks.selectedTabPreserved=true;
  if(web){try{result.web=await web.observe('after-e-realism')}catch(error){result.web={status:'RED',reason:error.message.split('; last snapshot')[0]}}}else result.web={status:'blocked',reason:'Web observer unavailable'};
  result.status='PASS';
 }catch(error){result.reason=error.message.split('; last snapshot')[0]}
 finally{if(owned)await client.send(requests.releaseRoomEnvironmentInputRequest(sessionId,{kind:'browser_tab',id:sameTabId})).catch(()=>{});await writeFile(out+'/drill-e-realism.json',JSON.stringify(result,null,2)+'\n',{mode:0o600})}
 return result;
}
