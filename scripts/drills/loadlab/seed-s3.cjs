// Agent-side setup for the S3 driver (40 Rooms, 300 notifications in about 10 s). createRoom(alias) makes one lab Room and returns
// { sessionId, attachmentId, rootId }; rpc is as in seed-s1. seedS3 spawns each Room's trace agent idle and hides the Room's own initial agent;
// start() sends the 40 prompts, so the notification bursts begin when the caller starts measuring.
const { parseTrace, filler } = require('../../../apps/kernel/src/provider/registry/dev_stub_adapter/trace_format.cjs')

async function seedS3({ rpc, createRoom, traces }) {
  const plans = new Map([...traces].map(([file, contents]) => {
    const plan = parseTrace(contents, file)
    return [plan.header.agent, plan]
  }))
  if (plans.size !== 40) throw new Error('expected the generated S3 trace set')
  const rooms = []
  for (let i = 0; i < 40; i++) {
    const alias = `room-${i + 1}`
    const plan = plans.get(alias)
    if (!plan) throw new Error(`S3 trace for ${alias} is missing`)
    const room = await createRoom(alias)
    const { AgentSpawned: { agent } } = await rpc({
      SpawnAgent: { session_id: room.sessionId, alias, provider: 'dev-stub', model: 'trace', permission_level: 'yolo', no_subscribe: true },
    })
    await rpc({ HideAgent: { session_id: room.sessionId, agent_id: room.rootId, hidden: true } })
    const notifications = plan.turns[0].filter(step => step.kind === 'notify').reduce((total, step) => total + step.count, 0)
    rooms.push({ ...room, alias, agentId: agent.id, notifications, prompt: filler(plan.turns[0][0].chars) })
  }

  async function start() {
    for (const room of rooms) {
      await rpc({ SubmitPrompt: { session_id: room.sessionId, attachment_id: room.attachmentId, target_agent_id: room.agentId, prompt: room.prompt } })
    }
  }

  return { rooms, start, expectedNotifications: rooms.reduce((total, room) => total + room.notifications, 0) }
}

module.exports = { seedS3 }
