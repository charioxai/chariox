// MP-08 / MP-09 / MP-10 / MP-11: owner-side driver for a Room agent that does account signups.
// Prompts dropped as ROOM_DIR/cmd/<agent-id>.<n>.prompt are submitted; provider output goes to events.jsonl,
// Room cards (human handoffs) to interactions.jsonl, agent/task state changes to agentstate.jsonl.
import {appendFile, mkdir, readdir, readFile, rename, writeFile} from 'node:fs/promises'
import {connect} from './rpc.mjs'

const dir = process.env.ROOM_DIR, sessionId = process.env.ROOM_SESSION
if (!dir || !sessionId) throw Error('Set ROOM_DIR (scratch outside the repository) and ROOM_SESSION')
await mkdir(`${dir}/cmd`, {recursive: true}); await mkdir(`${dir}/done`, {recursive: true})
const line = (file, value) => appendFile(`${dir}/${file}`, JSON.stringify({ts: new Date().toISOString(), ...value}) + '\n')
const log = message => appendFile(`${dir}/driver.log`, `${new Date().toISOString()} ${message}\n`)
await writeFile(`${dir}/driver.pid`, String(process.pid))

const open = new Map()
let lastState
async function track(peer) {
  const session = (await peer.send({ListSessions: {}})).SessionsListed.sessions.find(item => item.id === sessionId)
  const state = JSON.stringify({
    agents: session.agents.map(agent => [agent.alias, agent.state, agent.is_processing]),
    prompt: session.active_prompt?.id ?? null,
    tasks: (session.agent_tasks ?? []).filter(task => !['done', 'cancelled'].includes(task.state)).map(task => [task.task_id, task.state]),
  })
  if (state !== lastState) { lastState = state; await line('agentstate.jsonl', {state: JSON.parse(state)}) }
  const current = new Set()
  for (const card of session.active_interactions ?? []) {
    current.add(card.id)
    if (open.has(card.id)) continue
    open.set(card.id, Date.now())
    await line('interactions.jsonl', {event: 'appeared', requested_at: new Date(card.requested_at_ms || 0).toISOString(), id: card.id, kind: card.kind, title: card.title, message: String(card.message).slice(0, 400), choices: (card.choices ?? []).map(choice => choice.id)})
  }
  for (const [id, since] of [...open]) {
    if (current.has(id)) continue
    open.delete(id)
    await line('interactions.jsonl', {event: 'gone', id, seconds_open: Math.round((Date.now() - since) / 1000)})
  }
}

for (;;) {
  let peer
  try {
    peer = await connect()
    const attached = await peer.send({AttachToSession: {session_id: sessionId, client_id: 'room-driver', capability_level: 'FullTerminal'}})
    const attachment = attached.SessionAttached.attachment.id
    await log(`attached ${attachment}`)
    for (;;) {
      // An idle attachment is reaped, so the pump runs every two seconds.
      const output = await peer.send({PumpTerminalOutput: {session_id: sessionId, attachment_id: attachment}})
      for (const record of output.TerminalOutput.records) await line('events.jsonl', {rec: record})
      for (const file of (await readdir(`${dir}/cmd`)).filter(name => name.endsWith('.prompt')).sort()) {
        const [agent] = file.split('.')
        const prompt = await readFile(`${dir}/cmd/${file}`, 'utf8')
        await rename(`${dir}/cmd/${file}`, `${dir}/done/${file}`)
        await peer.send({SubmitPrompt: {session_id: sessionId, attachment_id: attachment, target_agent_id: agent, prompt, attachments: []}})
        await log(`submitted ${file}`)
      }
      await track(peer)
      await new Promise(resolve => setTimeout(resolve, 2000))
    }
  } catch (error) { await log(`error ${error.message.slice(0, 200)}`) }
  try { peer?.close() } catch { /* already closed */ }
  await new Promise(resolve => setTimeout(resolve, 5000))
}
