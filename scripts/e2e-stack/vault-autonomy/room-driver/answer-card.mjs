// MP-08 / MP-09 / MP-10 / MP-11: answer or list the open Room cards: node answer-card.mjs [<card-id> <choice-id>].
import {connect} from './rpc.mjs'

const sessionId = process.env.ROOM_SESSION
if (!sessionId) throw Error('Set ROOM_SESSION')
const peer = await connect()
const [id, choice] = process.argv.slice(2)
if (id && choice) {
  console.log(Object.keys(await peer.send({RespondToInteraction: {session_id: sessionId, interaction_id: id, choice_id: choice}}))[0])
} else {
  const session = (await peer.send({ListSessions: {}})).SessionsListed.sessions.find(item => item.id === sessionId)
  for (const card of session.active_interactions ?? []) console.log(card.id, card.title, (card.choices ?? []).map(item => item.id).join(','))
}
peer.close()
