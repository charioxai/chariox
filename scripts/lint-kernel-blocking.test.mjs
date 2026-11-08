import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { blockingViolations } from './lint-kernel-blocking.mjs'
import { lockAcrossAwait } from './lib/kernel-lock-across-await.mjs'
import { loopWaits } from './lib/kernel-loop-waits.mjs'
import { productionSource } from './lib/rust-production-source.mjs'

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

const lockRule = (source, allowed = []) => lockAcrossAwait(productionSource(source, path), path, allowed)
test('a std or parking_lot guard live at an await fails the gate', () => {
  assert.equal(lockRule('async fn f(&self) { let g = self.m.lock(); other().await; g.touch(); }').length, 1)
  assert.equal(lockRule('async fn f(&self) { let mut g = self.m.lock().unwrap_or_else(PoisonError::into_inner); other().await; }').length, 1)
  assert.equal(lockRule('async fn f(&self) { let g = self.m.read().expect("poisoned"); a().await; }').length, 1)
  assert.equal(lockRule('fn shutdown(&self, rt: Handle) { let g = self.m.lock()?; rt.block_on(async { drain().await }); }').length, 1)
})
test('values taken out of the guard, dropped guards, spawned blocks and async locks pass the gate', () => {
  assert.deepEqual(lockRule('async fn f(&self) { let v = self.m.lock().unwrap().clone(); other().await; }'), [])
  assert.deepEqual(lockRule('async fn f(&self) { let v = self.m.lock().unwrap_or_else(PoisonError::into_inner).get(k).cloned(); other().await; }'), [])
  assert.deepEqual(lockRule('async fn f(&self) { let g = self.m.lock(); g.touch(); drop(g); other().await; }'), [])
  assert.deepEqual(lockRule('async fn f(&self) { { let g = self.m.lock(); g.touch(); } other().await; }'), [])
  assert.deepEqual(lockRule('async fn f(&self) { let g = self.m.lock(); tokio::spawn(async move { other().await; }); g.touch(); }'), [])
  assert.deepEqual(lockRule('async fn f(&self) { let g = self.m.lock().await; other().await; }'), [])
  assert.deepEqual(lockRule('async fn f(&self) { let g = self.m.write().await; other().await; }'), [])
})
test('a reviewed guard site is admitted only with its path, name and initializer', () => {
  const source = 'async fn f(&self) { let g = self.m.lock(); other().await; }'
  const site = { path, guard: 'g', init: 'self.m.lock()', reason: 'test' }
  assert.deepEqual(lockRule(source, [site]), [])
  assert.equal(lockRule(source, [{ ...site, guard: 'h' }]).length, 1)
  assert.equal(lockRule(source, [{ ...site, path: 'apps/kernel/src/other.rs' }]).length, 1)
})

const schedulerPath = 'apps/kernel/src/runtime/state/agent_wake_scheduler.rs'
const loopRule = (source, allowed = []) => loopWaits(productionSource(source, schedulerPath), schedulerPath, ['run_agent_wake_scheduler'], allowed)
// The shape of agent_wake_scheduler.rs before the sweep moved to its own task: cleanup awaited inline in the loop.
const unboundedSweep = 'async fn run_agent_wake_scheduler(&self) { loop { self.end_orphaned_sub_agents().await; tokio::time::sleep(TICK).await; } }'
test('an unbounded await in the body of a scheduler loop fails the gate', () => {
  const hits = loopRule(unboundedSweep)
  assert.equal(hits.length, 1)
  assert.match(hits[0].rule, /unbounded `end_orphaned_sub_agents\(\.\.\)\.await` in loop `run_agent_wake_scheduler`/)
  assert.equal(blockingViolations(unboundedSweep, schedulerPath).filter(v => /unbounded/.test(v.rule)).length, 1)
})
test('bounded, spawned, pacing and reviewed awaits in a scheduler loop pass the gate', () => {
  assert.deepEqual(loopRule('async fn run_agent_wake_scheduler(&self) { loop { tokio::time::timeout(LIMIT, self.end_orphaned_sub_agents()).await; } }'), [])
  assert.deepEqual(loopRule('async fn run_agent_wake_scheduler(&self) { let mut sweep = JoinSet::new(); loop { sweep.spawn(async move { state.end_orphaned_sub_agents().await }); tokio::time::sleep(TICK).await; tokio::task::yield_now().await; } }'), [])
  assert.deepEqual(loopRule('async fn run_agent_wake_scheduler(&self) { loop { if let Err(e) = self.tick_deadlines(now).await {} } }', [{ path: schedulerPath, fn: 'run_agent_wake_scheduler', call: 'tick_deadlines', reason: 'test' }]), [])
  assert.equal(loopRule('async fn run_agent_wake_scheduler(&self) { loop { if let Err(e) = self.tick_deadlines(now).await {} } }').length, 1)
})
test('a loop function that was renamed or removed fails the gate instead of passing silently', () => {
  const hits = loopRule('async fn run_agent_wake_scheduler_v2(&self) { loop { sleep(TICK).await; } }')
  assert.equal(hits.length, 1)
  assert.match(hits[0].rule, /no longer exists/)
})
