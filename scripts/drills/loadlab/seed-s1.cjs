// Agent-side setup for the S1 driver. rpc sends an existing kernel request and
// returns its response object, throwing on refusal. The caller owns the lab Room.
const { setTimeout: delay } = require('node:timers/promises')
const { parseTrace, filler } = require('../../../apps/kernel/src/provider/registry/dev_stub_adapter/trace_format.cjs')

async function seedS1({ rpc, sessionId, attachmentId, traces, signal = AbortSignal.timeout(660_000) }) {
  const plans = new Map([...traces].map(([file, contents]) => {
    const plan = parseTrace(contents, file)
    return [plan.header.agent, plan]
  }))
  const root = plans.get('orch')
  if (!root || plans.size !== 41) throw new Error('expected the generated S1 trace set')
  const { AgentSpawned: { agent } } = await rpc({
    SpawnAgent: { session_id: sessionId, alias: 'orch', provider: 'dev-stub', model: 'trace', permission_level: 'required', no_subscribe: true },
  })
  await rpc({
    SubmitPrompt: { session_id: sessionId, attachment_id: attachmentId, target_agent_id: agent.id, prompt: filler(root.turns[0][0].chars) },
  })

  const readTree = async () => (await rpc({
    ListDescendants: { session_id: sessionId, agent_id: agent.id },
  })).AgentTree.tree

  async function until(read, ready, description) {
    let value
    while (!signal.aborted) {
      value = await read()
      if (ready(value)) return value
      await delay(200)
    }
    const states = value?.nodes?.map(({ alias, state, outcome }) => `${alias}=${state}/${outcome ?? ""}`)
    throw new Error(`S1 timed out waiting for ${description}${states ? ` (${states.join(", ")})` : ""}`)
  }

  const tree = await until(readTree, tree => tree.nodes.length === 40, '40 real sub-agents')
  const histories = tree.nodes.filter(node => plans.get(node.alias)?.header.notes === 'done')
  if (histories.length !== 22 || Math.max(...tree.nodes.map(node => node.depth)) !== 2) {
    throw new Error('S1 did not create 22 history agents in a tree two levels deep')
  }

  async function readTurns(agentIds) {
    return (await rpc({
      GetSessionHistoryOutline: { session_id: sessionId, agent_ids: agentIds, latest_prompt_count: 1 },
    })).SessionHistoryOutline.agents
  }

  async function collect(ids, round, expectedIds) {
    const results = await until(() => readTurns(ids), rows => rows.length === ids.length && rows.every(row => {
      const turn = row.turns[0]
      return turn && turn.lifecycle !== 'open' && (!expectedIds || turn.prompt_id === expectedIds.get(row.agent_id))
    }), `history round ${round} to settle`)
    for (const { agent_id, turns: [turn] } of results) {
      if (turn.lifecycle !== 'completed') throw new Error(`S1 history turn ended ${turn.lifecycle}: ${agent_id}`)
    }
  }

  await collect(histories.map(node => node.agent_id), 1)
  const rounds = Math.max(...histories.map(node => plans.get(node.alias).turns.length))
  for (let round = 1; round < rounds; round++) {
    signal.throwIfAborted()
    const prompts = histories.flatMap(node => {
      const turn = plans.get(node.alias).turns[round]
      return turn ? [{ target_agent_id: node.agent_id, prompt: filler(turn[0].chars) }] : []
    })
    const { PromptsSubmitted: { results } } = await rpc({
      SubmitPrompts: { session_id: sessionId, attachment_id: attachmentId, max_concurrency: 8, prompts },
    })
    if (results.length !== prompts.length) throw new Error('S1 did not submit the full next history round')
    const submitted = new Map(results.map(({ agent_id, outcome }) => [
      agent_id, (outcome.Started ?? outcome.Queued).prompt.id,
    ]))
    await collect([...submitted.keys()], round + 1, submitted)
  }

  const counts = new Map()
  for (const { agent_id, alias } of histories) {
    let before
    let count = 0
    do {
      signal.throwIfAborted()
      const { AgentTranscriptPeek: { peek } } = await rpc({
        PeekAgentTranscript: { session_id: sessionId, agent_id, limit: 50, before },
      })
      count += peek.entries.length
      before = peek.before
      if (count > 2_000) throw new Error(`S1 history for ${alias} exceeds 2,000 entries`)
    } while (before)
    if (count < 200) throw new Error(`S1 history for ${alias} has ${count} entries`)
    counts.set(agent_id, count)
  }
  const settled = await until(readTree, ({ nodes }) =>
    nodes.filter(node => node.state === 'running').length === 12 &&
    nodes.filter(node => node.state === 'waiting').length === 6 &&
    nodes.filter(node => node.state === 'idle' && node.outcome === 'completed').length === 22,
  '12 running, 6 waiting, and 22 completed sub-agents')
  return { rootAgentId: agent.id, tree: settled, historyEntries: Object.fromEntries(counts) }
}

module.exports = { seedS1 }
