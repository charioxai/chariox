// MP-08/MP-10: post-workflow projections, independent of business-result acceptance.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {roomActionNoticePattern,automationNoticeTexts} from './room-tui-notices.mjs';

export async function proveExtraClients(input, baseline, out) {
 const {client,requests,sessionId,provider,localAutomation,remoteAutomation,web,waitFor}=input;
 const result={mp_items:['MP-08','MP-10'],status:'RED',baseline};
 try {
  const history=(await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId,null,100))).RoomEnvironmentActionHistoryListed.page.actions;
  const actions=history.filter(a=>a.sequence>baseline&&a.actor_id==='agent:'+provider.agentId&&a.mode==='browser'&&a.state==='completed');
  assert.ok(actions.length,'MP-08/MP-10 workflow produced no Browser actions');
  result.actions=actions.map(a=>({id:a.action_id,sequence:a.sequence,kind:a.kind}));
  for(const [name,tui] of [['localTui',localAutomation],['remoteTui',remoteAutomation]]) {
   assert.ok(tui,'MP-08/MP-10 workflow TUI unavailable');
   for(const action of actions.slice(0,3)) await waitFor(async()=>{
    const s=await tui.send('snapshot');
    return automationNoticeTexts(s).some(text=>roomActionNoticePattern(action).test(text));
   },15000,'MP-08/MP-10 workflow TUI attribution absent');
   result[name]=true;
  }
  assert.ok(web,'MP-08/MP-10 workflow Web observer unavailable');
  const env=(await client.send(requests.getRoomEnvironmentStateRequest(sessionId))).RoomEnvironmentState.environment;
  result.web=await web.observe('return-view');
  assert.ok(result.web.samples.every(s=>s.environmentId===env.environment_id&&s.tabId===env.focused_tab_id&&s.canvasCount===1&&s.connected&&s.ready),'MP-08/MP-10 workflow Web changed Room/tab or lost stream');
  result.status='PASS';
 } catch(error) {result.reason=error.message.split('; last snapshot')[0];}
 await writeFile(out+'/workflow-client-proof.json',JSON.stringify(result,null,2)+'\n',{mode:0o600});
 return result;
}
