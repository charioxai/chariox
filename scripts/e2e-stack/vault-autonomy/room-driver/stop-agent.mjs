// MP-08 / MP-09 / MP-10 / MP-11: cancel an agent's running and queued prompts so it stops touching the browser:
// node stop-agent.mjs <agent-id> <attachment-id>. Parked tasks and wakes stay: a card answer or a timer can still wake it.
import {connect} from './rpc.mjs'

const sessionId = process.env.ROOM_SESSION
const [agent, attachment] = process.argv.slice(2)
if (!sessionId || !attachment) throw Error('Set ROOM_SESSION and pass: agent-id attachment-id')
const peer = await connect()
const session = (await peer.send({ListSessions: {}})).SessionsListed.sessions.find(item => item.id === sessionId)
for (const queued of session.queued_prompts ?? []) {
  if (queued.target_agent_id !== agent) continue
  await peer.send({CancelQueuedPrompt: {session_id: sessionId, attachment_id: attachment, target_agent_id: agent, prompt_id: queued.id}})
  console.log('cancelled queued', queued.id)
}
console.log(Object.keys(await peer.send({CancelActivePrompt: {session_id: sessionId, attachment_id: attachment, target_agent_id: agent}}))[0])
peer.close()
