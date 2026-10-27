// MP-08 / MP-10: private, opt-in drill control; never a runtime protocol surface.
import assert from 'node:assert/strict'
import { lstat, readFile, rename, writeFile } from 'node:fs/promises'
import path from 'node:path'

export function createRoomWebFaultControl({ directory, handlers }) {
  assert.ok(path.isAbsolute(directory))
  let lastId = 0
  return async () => {
    const file = path.join(directory, 'fault-command.json')
    let entry
    try { entry = await lstat(file) } catch (error) { if (error.code === 'ENOENT') return; throw error }
    assert.ok(entry.isFile() && !entry.isSymbolicLink() && entry.size <= 4096)
    assert.equal(entry.uid, process.getuid())
    assert.equal(entry.mode & 0o777, 0o600)
    const command = JSON.parse(await readFile(file, 'utf8'))
    assert.ok(Number.isSafeInteger(command.id) && command.id > 0)
    if (command.id <= lastId) return
    assert.deepEqual(Object.keys(command).sort(), ['id', 'operation'])
    assert.ok(Object.hasOwn(handlers, command.operation), 'unsupported fault operation')
    lastId = command.id
    const started = Date.now()
    let result
    try { result = { ok: true, data: await handlers[command.operation]() } }
    catch (error) { result = { ok: false, errorClass: error.name, error: String(error.message).slice(0, 500) } }
    const temporary = path.join(directory, 'fault-response.tmp')
    await writeFile(temporary, JSON.stringify({ id: command.id, operation: command.operation, elapsedMs: Date.now() - started, ...result }), { mode: 0o600 })
    await rename(temporary, path.join(directory, 'fault-response.json'))
  }
}
