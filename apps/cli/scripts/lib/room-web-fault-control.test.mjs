// MP-08 / MP-10: duplicate polling must never replay a destructive fault.
import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdtemp, readFile, rm, writeFile, chmod } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { createRoomWebFaultControl } from './room-web-fault-control.mjs'

test('fault polling executes once, rejects unknown controls and enforces private files', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'webfault-control-'))
  try {
    let calls = 0
    const pump = createRoomWebFaultControl({ directory, handlers: { stop: async () => ++calls } })
    const file = path.join(directory, 'fault-command.json')
    await writeFile(file, JSON.stringify({ id: 1, operation: 'stop' }), { mode: 0o600 })
    await pump(); await pump()
    assert.equal(calls, 1)
    assert.deepEqual(JSON.parse(await readFile(path.join(directory, 'fault-response.json'))).data, 1)
    await writeFile(file, JSON.stringify({ id: 2, operation: 'shell' }))
    await assert.rejects(pump(), /unsupported/)
    assert.equal(calls, 1)
    await writeFile(file, JSON.stringify({ id: 2, operation: 'stop' }))
    await chmod(file, 0o644)
    await assert.rejects(pump())
    assert.equal(calls, 1)
  } finally { await rm(directory, { recursive: true, force: true }) }
})
