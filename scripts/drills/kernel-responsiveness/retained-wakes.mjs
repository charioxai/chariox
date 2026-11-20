// Seed only the stopped, disposable drill kernel. Each former agent keeps at
// most the production retention of 64 finished wakes; command data is synthetic.
import assert from 'node:assert/strict'
import { DatabaseSync } from 'node:sqlite'
import { join } from 'node:path'

export function seedRetainedWakes(root, room, count, commandBytes) {
  assert(root.startsWith('/root/.chariox/dev/'), 'isolated drill state required')
  assert(Number.isSafeInteger(count) && count > 0)
  assert(Number.isSafeInteger(commandBytes) && commandBytes >= 0)
  const db = new DatabaseSync(join(root, 'chariox/state.db'))
  let payloadBytes = 0
  try {
    db.exec('BEGIN')
    const insert = db.prepare('INSERT INTO agent_wakes(id,task_id,room_id,agent_id,payload) VALUES(?,?,?,?,?)')
    for (let i = 0; i < count; i++) {
      const id = `kernelstall-finished-${i}`, task = `kernelstall-old-turn-${i}`, agent = `kernelstall-former-agent-${Math.floor(i / 64)}`
      const payload = JSON.stringify({ id, task_id: task, room_id: room, agent_id: agent,
        registration_id: `completion-${id}`, kind: 'process', label: 'finished cargo build', state: 'exited',
        created_at_ms: Date.now() - 60000, verified_at_ms: Date.now() - 60000,
        command: ['cargo', 'build', 'x'.repeat(commandBytes)], pid: null, exit_code: 0,
        summary: 'finished', detail: null, fire_count: 0, missed_fires: 0 })
      insert.run(id, task, room, agent, payload)
      payloadBytes += Buffer.byteLength(payload)
    }
    db.exec('COMMIT')
  } finally { db.close() }
  return { count, former_agents: Math.ceil(count / 64), command_bytes: commandBytes, payload_bytes: payloadBytes }
}
