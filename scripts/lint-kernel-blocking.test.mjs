import assert from 'node:assert/strict'
import test from 'node:test'
import { blockingViolations } from './lint-kernel-blocking.mjs'

const path = 'apps/kernel/src/provider/example.rs'
test('raw child waits and process output fail the gate', () => {
  assert.equal(blockingViolations('fn stop() { child.wait(); command.output(); }', path).length, 2)
})
test('comments, strings and test-only items do not enter production checks', () => {
  assert.deepEqual(blockingViolations('// child.wait();\nfn run() { let x = "command.output()"; }\n#[cfg(test)] mod tests { fn test() { child.wait(); } }', path), [])
})
test('production environment lock and store-held connect fail the gate', () => {
  assert.equal(blockingViolations('fn start() { env_lock::lock(); self.write().start_run_provider_only(request); }', 'apps/kernel/src/provider/service/store.rs').length, 2)
})
test('bounded methods and helper implementation are admitted', () => {
  assert.deepEqual(blockingViolations('fn stop() { child.bounded_wait(); command.bounded_output(); }', path), [])
  assert.deepEqual(blockingViolations('fn output() { child.wait(); }', 'apps/kernel/src/runtime/bounded_process.rs'), [])
})

test('receipt modules do not exempt new child waits or process output', () => {
  assert.equal(blockingViolations('fn stop() { child.wait(); command.output(); }', 'apps/kernel/src/room_terminal/store.rs').length, 2)
})
