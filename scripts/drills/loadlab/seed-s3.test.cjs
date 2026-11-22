const assert = require('node:assert/strict')
const test = require('node:test')
const { generate } = require('./generate.cjs')
const { seedS3 } = require('./seed-s3.cjs')

test('S3 seeding makes 40 Rooms with an idle trace agent each; start() prompts them and 300 notifications are planned', async () => {
  const requests = []
  let rooms = 0
  let agents = 0
  const rpc = async request => {
    requests.push(request)
    if (request.SpawnAgent) return { AgentSpawned: { agent: { id: `agent-${++agents}` } } }
    return {}
  }
  const createRoom = async alias => ({ sessionId: `session-${alias}`, attachmentId: `attachment-${++rooms}`, rootId: `root-${rooms}` })
  const seeded = await seedS3({ rpc, createRoom, traces: generate('s3', 42) })
  assert.equal(seeded.rooms.length, 40)
  assert.equal(seeded.expectedNotifications, 300)
  assert.equal(seeded.rooms.filter(room => room.notifications > 0).length, 8)
  assert.equal(requests.filter(r => r.HideAgent).length, 40)
  assert.equal(requests.filter(r => r.SubmitPrompt).length, 0)
  await seeded.start()
  assert.equal(requests.filter(r => r.SubmitPrompt).length, 40)
})
