// MP-08/MP-10: actual production Web input plus kernel/TUI/physical verification.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {readDrillEActionHistory} from '../live-browser-computer-drill-e.mjs';
import {roomActionNoticePattern} from './room-tui-notices.mjs';
export async function runWebTakeoverProof(i){
 const {client,requests,sessionId,web,waitFor,waitForBrowserText,waitForLocalNotice,waitForRemoteNotice,publicEvidenceRoot}=i;
 const r={mp_items:['MP-08','MP-10'],status:'RED'};
 try{
  const before=await readDrillEActionHistory(client,requests,sessionId),baseline=Math.max(...before.map(a=>a.sequence));
  r.web=await web.observe('takeover');
  const env=(await client.send(requests.getRoomEnvironmentStateRequest(sessionId))).RoomEnvironmentState.environment;
  const humans=new Set(env.actors.filter(a=>a.kind==='human').map(a=>a.actor_id));
  const a=await waitFor(async()=>{const history=await readDrillEActionHistory(client,requests,sessionId);return history.find(a=>a.sequence>baseline&&humans.has(a.actor_id)&&a.kind==='pointer_click'&&a.state==='completed')},15000,'MP-08/MP-10 Web human native pointer action missing');
  await waitForBrowserText('POINTER_CLICK_COUNT=2',15000,'MP-08/MP-10 Web human physical click effect missing');
  await Promise.all([waitForLocalNotice(roomActionNoticePattern(a)),waitForRemoteNotice(roomActionNoticePattern(a))]);
  r.action={actionId:a.action_id,sequence:a.sequence,actorId:a.actor_id,kind:a.kind,state:a.state};r.physicalEffect='POINTER_CLICK_COUNT=2';r.localTui=true;r.remoteTui=true;r.status='PASS';
 }catch(error){r.reason=error.message.split('; last snapshot')[0];throw error}
 finally{await writeFile(publicEvidenceRoot+'/web-takeover-proof.json',JSON.stringify(r,null,2)+'\n',{mode:0o600})}
 return r;
}
