import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
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

test('the gate runs without ripgrep on PATH and still flags a planted violation', () => {
  const script = fileURLToPath(new URL('./lint-kernel-blocking.mjs', import.meta.url))
  const root = mkdtempSync(join(tmpdir(), 'blocking-lint-'))
  try {
    mkdirSync(join(root, 'apps/kernel/src/provider'), { recursive: true })
    writeFileSync(join(root, 'apps/kernel/src/provider/clean.rs'), 'fn run() {}\n')
    const run = () => spawnSync(process.execPath, [script], { cwd: root, encoding: 'utf8', env: { PATH: join(root, 'no-binaries') } })
    const clean = run()
    assert.ok(clean.status === 0, `clean tree: ${clean.stderr}`)
    assert.match(clean.stdout, /1 files checked, 0 violations/)
    writeFileSync(join(root, 'apps/kernel/src/provider/planted.rs'), 'fn stop() { child.wait(); }\n')
    const planted = run()
    assert.ok(planted.status === 1, `planted tree exit ${planted.status}`)
    assert.match(planted.stderr, /planted\.rs:1: raw subprocess call/)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
