// Agent-side setup for the S2 driver (one Room, 50 agents). Like seed-s1, rpc sends an existing kernel
// request and returns its response object, throwing on refusal; the caller owns the lab Room.
// seedS2 only creates the 50 idle agents. start() sends their prompts, so the streaming text, the six
// terminals and the three permission cards begin when the caller starts measuring, not minutes before.
const { setTimeout: delay } = require('node:timers/promises')
const { parseTrace, filler } = require('../../../apps/kernel/src/provider/registry/dev_stub_adapter/trace_format.cjs')

const aliasOf = index => `room-agent-${index + 1}`

function classify(plan) {
  const kinds = new Set(plan.turns[0].map(step => step.kind))
  if (kinds.has('terminal')) return 'terminal'
  if (kinds.has('permission')) return 'permission'
  return plan.turns[0].some(step => step.kind === 'text' && step.stream_ms > 0) ? 'streaming' : 'idle'
}

async function seedS2({ rpc, sessionId, attachmentId, traces, signal = AbortSignal.timeout(300_000) }) {
  const plans = new Map([...traces].map(([file, contents]) => {
    const plan = parseTrace(contents, file)
    return [plan.header.agent, plan]
  }))
  if (plans.size !== 50) throw new Error('expected the generated S2 trace set')
  const agents = []
  for (let i = 0; i < 50; i++) {
    const alias = aliasOf(i)
    const plan = plans.get(alias)
    if (!plan) throw new Error(`S2 trace for ${alias} is missing`)
    // Only the three permission agents may wait for the owner: opening a terminal is itself a gated action, and an agent that waits
    // for that card never prints, so the other 47 run with their permissions granted.
    const kind = classify(plan)
    const { AgentSpawned: { agent } } = await rpc({
      SpawnAgent: { session_id: sessionId, alias, provider: 'dev-stub', model: 'trace', permission_level: kind === 'permission' ? 'required' : 'yolo', no_subscribe: true },
    })
    agents.push({ id: agent.id, alias, kind, prompt: filler(plan.turns[0][0].chars) })
  }

  const listed = async () => (await rpc({ ListAgents: { session_id: sessionId } })).AgentsListed.agents

  async function start() {
    const { PromptsSubmitted: { results } } = await rpc({
      SubmitPrompts: {
        session_id: sessionId, attachment_id: attachmentId, max_concurrency: 8,
        prompts: agents.map(agent => ({ target_agent_id: agent.id, prompt: agent.prompt })),
      },
    })
    if (results.length !== agents.length) throw new Error('S2 did not submit all 50 prompts')
    while (!signal.aborted) {
      const rows = await listed()
      const byId = new Map(rows.map(row => [row.id, row]))
      if (agents.every(agent => byId.has(agent.id))) return byId
      await delay(200)
    }
    throw new Error('S2 timed out waiting for 50 Room agents')
  }

  return { agents, start, listed }
}

module.exports = { seedS2 }
