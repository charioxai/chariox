// MP-08/MP-10: actual production Web input plus kernel/TUI/physical verification.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {readDrillEActionHistory} from '../live-browser-computer-drill-e.mjs';
import {roomActionNoticePattern} from './room-tui-notices.mjs';
export async function runWebTakeoverProof(i){
 const {client,requests,sessionId,web,waitFor,waitForBrowserText,waitForLocalNotice,waitForRemoteNotice,publicEvidenceRoot,proofLabel}=i;
 const r={mp_items:['MP-08','MP-10'],status:'RED'};
 try{
  const before=await readDrillEActionHistory(client,requests,sessionId),baseline=Math.max(0,...before.map(a=>a.sequence));
  const text=await waitForBrowserText('POINTER_CLICK_COUNT=',15000,'MP-08/MP-10 fixture counter unavailable before Web takeover');
  const count=/POINTER_CLICK_COUNT=(\d+)/.exec(text);assert.ok(count,'MP-08/MP-10 fixture counter malformed');r.beforeClickCount=Number(count[1]);r.expectedClickCount=r.beforeClickCount+1;
  r.web=await web.observe('takeover');
  const env=(await client.send(requests.getRoomEnvironmentStateRequest(sessionId))).RoomEnvironmentState.environment;
  const humans=new Set(env.actors.filter(a=>a.kind==='human').map(a=>a.actor_id));
  const a=await waitFor(async()=>{const history=await readDrillEActionHistory(client,requests,sessionId);return history.find(a=>a.sequence>baseline&&humans.has(a.actor_id)&&a.kind==='pointer_click'&&a.state==='completed')},15000,'MP-08/MP-10 Web human native pointer action missing');
  r.action={actionId:a.action_id,sequence:a.sequence,actorId:a.actor_id,kind:a.kind,state:a.state};
  const after=await waitForBrowserText('POINTER_CLICK_COUNT='+r.expectedClickCount,15000,'MP-08/MP-10 Web human physical click effect missing');
  assert.equal(Number(/POINTER_CLICK_COUNT=(\d+)/.exec(after)?.[1]),r.expectedClickCount,'MP-08/MP-10 Web input must apply exactly once');
  await Promise.all([waitForLocalNotice(roomActionNoticePattern(a)),waitForRemoteNotice(roomActionNoticePattern(a))]);
  r.action={actionId:a.action_id,sequence:a.sequence,actorId:a.actor_id,kind:a.kind,state:a.state};r.physicalEffect='POINTER_CLICK_COUNT='+r.expectedClickCount;r.localTui=true;r.remoteTui=true;r.status='PASS';
 }catch(error){r.reason=error.message.split('; last snapshot')[0];throw error}
 finally{await writeFile(publicEvidenceRoot+(proofLabel==='return'?'/web-return-takeover-proof.json':'/web-takeover-proof.json'),JSON.stringify(r,null,2)+'\n',{mode:0o600})}
 return r;
}
