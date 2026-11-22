const assert = require('node:assert/strict')
const test = require('node:test')
const { generate } = require('./generate.cjs')
const { seedS2 } = require('./seed-s2.cjs')

test('S2 seeding spawns 50 idle agents; only the three permission agents may wait for the owner', async () => {
  const requests = []
  let count = 0
  const rpc = async request => {
    requests.push(request)
    if (request.SpawnAgent) return { AgentSpawned: { agent: { id: `agent-${++count}` } } }
    if (request.SubmitPrompts) return { PromptsSubmitted: { results: request.SubmitPrompts.prompts.map((_, index) => ({ index })) } }
    if (request.ListAgents) return { AgentsListed: { agents: Array.from({ length: 50 }, (_, i) => ({ id: `agent-${i + 1}` })) } }
    throw new Error(`unexpected ${Object.keys(request)[0]}`)
  }
  const seeded = await seedS2({ rpc, sessionId: 's', attachmentId: 'a', traces: generate('s2', 42) })
  assert.equal(seeded.agents.length, 50)
  assert.equal(requests.filter(r => r.SubmitPrompts).length, 0, 'no prompt before start()')
  const level = kind => requests.filter(r => r.SpawnAgent && seeded.agents.find(a => a.alias === r.SpawnAgent.alias).kind === kind).map(r => r.SpawnAgent.permission_level)
  assert.deepEqual(level('permission'), ['required', 'required', 'required'])
  assert.ok(['streaming', 'terminal', 'idle'].every(kind => level(kind).every(l => l === 'yolo')))
  assert.deepEqual(['streaming', 'terminal', 'permission', 'idle'].map(kind => seeded.agents.filter(a => a.kind === kind).length), [10, 6, 3, 31])
  await seeded.start()
  assert.equal(requests.find(r => r.SubmitPrompts).SubmitPrompts.prompts.length, 50)
})
