import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { cpSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
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

// Registry locks on the runtime thread: a synchronous lock taken by an entry point that is called from async code
// waits behind whatever holds it (healthnext r2: the runtime thread stuck in remove_provider_pty_process -> unbind_key).
const runtimePath = 'apps/kernel/src/runtime/state/example.rs'
const registryHits = source => blockingViolations(source, runtimePath).filter(v => /registry lock/.test(v.rule))
test('a registry-locking call inside an async body or an inline app side effect fails the gate', () => {
  assert.equal(registryHits('async fn f(&self) { crate::runtime::process_resources::has_key(&key); }').length, 1)
  assert.equal(registryHits('async fn f(&self) { domain.unbind_key(&key); }').length, 1)
  const inline = registryHits('async fn f(&self) { self.with_app_side_effect(|app| remove_provider_pty_process(app, id)).await; }')
  assert.equal(inline.length, 1)
  assert.match(inline[0].rule, /`remove_provider_pty_process\(\.\.\)` .*runtime thread/)
  assert.equal(registryHits('async fn f(&self) { self.with_app_side_effect(|app| ProviderLaunchProcessRuntime::new(app).remove_run(id)).await; }').length, 1)
})
test('registry-locking calls on a blocking thread or in a plain sync fn pass the gate', () => {
  assert.deepEqual(registryHits('fn remove(&self) { self.unbind_key(&key); }'), [])
  assert.deepEqual(registryHits('async fn f(&self) { tokio::task::spawn_blocking(move || domain.unbind_key(&key)).await; }'), [])
  assert.deepEqual(registryHits('async fn f(&self) { self.with_app_side_effect_blocking(move |app| remove_provider_pty_process(app, id)).await; }'), [])
  assert.deepEqual(registryHits('async fn f(&self) { let id = has_key_label(&key); other_has_key(&key); }'), [])
})

// End to end: the gate as `pnpm lint` runs it, on a throwaway tree that carries its own copy of the scripts.
function fixtureRoot(sources, allowed) {
  const root = mkdtempSync(join(tmpdir(), 'registry-lint-'))
  const scripts = fileURLToPath(new URL('.', import.meta.url))
  mkdirSync(join(root, 'scripts'), { recursive: true })
  for (const entry of readdirSync(scripts)) if (/^kernel-.*\.json$|^lint-kernel-blocking\.mjs$/.test(entry)) cpSync(join(scripts, entry), join(root, 'scripts', entry))
  cpSync(join(scripts, 'lib'), join(root, 'scripts/lib'), { recursive: true })
  if (allowed) writeFileSync(join(root, 'scripts/kernel-registry-lock-calls.json'), JSON.stringify(allowed))
  for (const [file, source] of Object.entries(sources)) {
    mkdirSync(join(root, file, '..'), { recursive: true })
    writeFileSync(join(root, file), source)
  }
  return root
}
const runGate = root => spawnSync(process.execPath, [join(root, 'scripts/lint-kernel-blocking.mjs')], { cwd: root, encoding: 'utf8' })
const planted = 'async fn settle(&self) { self.with_app_side_effect(|app| remove_provider_pty_process(app, id)).await; }\n'
test('the gate fails a planted registry lock on the runtime thread, passes it once reviewed, and passes a clean tree', () => {
  const clean = fixtureRoot({ [runtimePath]: 'async fn settle(&self) { other().await; }\n' })
  const dirty = fixtureRoot({ [runtimePath]: planted })
  const reviewed = fixtureRoot({ [runtimePath]: planted }, [{ path: runtimePath, fn: 'settle', call: 'remove_provider_pty_process', reason: 'test: known, tracked' }])
  try {
    const cleanRun = runGate(clean)
    assert.ok(cleanRun.status === 0, `clean tree: ${cleanRun.stderr}`)
    assert.match(cleanRun.stdout, /0 violations/)
    const dirtyRun = runGate(dirty)
    assert.ok(dirtyRun.status === 1, `planted tree exit ${dirtyRun.status}: ${dirtyRun.stdout}`)
    assert.match(dirtyRun.stderr, /example\.rs:1: .*registry lock/)
    const reviewedRun = runGate(reviewed)
    assert.ok(reviewedRun.status === 0, `reviewed tree: ${reviewedRun.stderr}`)
  } finally {
    for (const root of [clean, dirty, reviewed]) rmSync(root, { recursive: true, force: true })
  }
})
test('a reviewed registry lock site is keyed by path, enclosing fn and call', () => {
  const site = { path: runtimePath, fn: 'settle', call: 'remove_provider_pty_process', reason: 'test' }
  const roots = [{ ...site, fn: 'other' }, { ...site, call: 'unbind_key' }, { ...site, path: 'apps/kernel/src/other.rs' }].map(entry => fixtureRoot({ [runtimePath]: planted }, [entry]))
  try {
    for (const root of roots) assert.ok(runGate(root).status === 1, 'a mismatched entry must not admit the site')
  } finally {
    for (const root of roots) rmSync(root, { recursive: true, force: true })
  }
})
test('a registry lock entry point that was renamed or no longer takes the lock fails the gate', () => {
  const definition = 'apps/kernel/src/runtime/process_resources.rs'
  const renamed = blockingViolations('pub(crate) fn unbind_key_v2(&self, key: &str) { registry().lock(); }', definition)
  assert.equal(renamed.filter(v => /unbind_key.*no longer/.test(v.rule)).length, 1)
  const unlocked = blockingViolations('pub(crate) fn unbind_key(&self, key: &str) { forget(key); }', definition)
  assert.equal(unlocked.filter(v => /unbind_key.*no longer takes `registry\(\)`/.test(v.rule)).length, 1)
  assert.deepEqual(blockingViolations('pub(crate) fn unbind_key(&self, key: &str) { registry().lock(); }', definition).filter(v => /unbind_key/.test(v.rule)), [])
})

// Entry points the healthguard review found missing from the table: `end_owner` takes the registry mutex itself,
// `keys_for_session` / `for_session` reach it through listed_keys / observed_keys_for_session, and the sweep of
// process_resources/ adds every other pub / pub(crate) function that locks it directly or through a private helper.
const processResources = 'apps/kernel/src/runtime/process_resources.rs'
const processControl = 'apps/kernel/src/runtime/process_resources/control.rs'
const handover = 'apps/kernel/src/runtime/process_resources/handover.rs'
const projection = 'apps/kernel/src/runtime/process_resources/projection.rs'
const launch = 'apps/kernel/src/runtime/process_resources/launch.rs'
const ptyManager = 'apps/kernel/src/pty/manager.rs'
const viaPath = (call, args) => `crate::runtime::process_resources::${call}(${args})`
const sweptEntryPoints = [
  { call: 'end_owner', defined: processControl, via: 'registry()', plant: viaPath('end_owner', 'None, None') },
  { call: 'keys_for_session', defined: projection, via: 'listed_keys(', plant: viaPath('keys_for_session', 'id, &providers, &terminals') },
  { call: 'for_session', defined: projection, via: 'observed_keys_for_session(', plant: viaPath('for_session', 'id, &providers, &terminals') },
  { call: 'stop_released', defined: processControl, via: 'released_keys(', plant: viaPath('stop_released', 'Some(id), None') },
  { call: 'hand_over_released', defined: handover, via: 'registry()', plant: viaPath('hand_over_released', '') },
  { call: 'adopt_released', defined: handover, via: 'start_released(', plant: viaPath('adopt_released', 'jobs') },
  { call: 'bind_terminal_for_key', defined: processResources, via: 'with_submissions_for_key(', plant: viaPath('bind_terminal_for_key', 'key, true') },
  { call: 'line_started_for_key', defined: processResources, via: 'with_submissions_for_key(', plant: viaPath('line_started_for_key', 'key') },
  { call: 'line_finished_for_key', defined: processResources, via: 'with_submissions_for_key(', plant: viaPath('line_finished_for_key', 'key, &jobs') },
  { call: 'release', defined: processResources, via: 'registry()', plant: 'self.resources.release(id, owner, None)' },
  { call: 'observe', defined: launch, via: 'samples_for_key(', plant: 'permit.observe()' },
  { call: 'release_exited_process_jobs', defined: ptyManager, via: 'release(', plant: 'pty.release_exited_process_jobs(&run_id, id, owner, None)' },
]
test('a planted async call to each swept registry lock entry point fails the gate', () => {
  for (const { call, plant } of sweptEntryPoints) {
    const hits = registryHits(`async fn f(&self) { ${plant}; }`)
    assert.equal(hits.length, 1, `${call} planted in an async fn`)
    assert.match(hits[0].rule, new RegExp(`\`${call}\\(\\.\\.\\)\``))
    assert.equal(registryHits(`async fn f(&self) { self.with_app_side_effect(|app| ${plant}).await; }`).length, 1, `${call} planted in an inline app side effect`)
  }
})
test('a swept entry point on a blocking thread or in a plain sync fn passes the gate', () => {
  for (const { call, plant } of sweptEntryPoints) {
    assert.deepEqual(registryHits(`fn f(&self) { ${plant}; }`), [], `${call} in a sync fn`)
    assert.deepEqual(registryHits(`async fn f(&self) { tokio::task::spawn_blocking(move || ${plant}).await; }`), [], `${call} in spawn_blocking`)
  }
})
test('a reviewed site admits a swept entry point only with its path, enclosing fn and call', () => {
  const { call, plant } = sweptEntryPoints[0]
  const source = `async fn settle(&self) { ${plant}; }\n`
  const site = { path: runtimePath, fn: 'settle', call, reason: 'known, tracked' }
  assert.equal(registryHits(source).length, 1)
  const roots = [site, { ...site, fn: 'other' }, { ...site, call: 'stop_released' }].map(entry => fixtureRoot({ [runtimePath]: source }, [entry]))
  try {
    assert.ok(runGate(roots[0]).status === 0, 'the matching entry admits the site')
    assert.ok(runGate(roots[1]).status === 1, 'another fn is not admitted')
    assert.ok(runGate(roots[2]).status === 1, 'another call is not admitted')
  } finally {
    for (const root of roots) rmSync(root, { recursive: true, force: true })
  }
})
test('names shared with unrelated types are flagged only on the registry-locking receiver', () => {
  assert.deepEqual(registryHits('async fn f(&self) { pump.release(); gate.release(); engine.release(); modal.observe(); }'), [])
  assert.deepEqual(registryHits('async fn f(&self) { let id = for_session_label(&key); other_end_owner(&key); }'), [])
})
test('a swept entry point that was renamed or no longer takes the lock fails the gate', () => {
  for (const { call, defined, via } of sweptEntryPoints) {
    const renamed = blockingViolations(`pub(crate) fn ${call}_v2(&self) { ${via}; }`, defined)
    assert.equal(renamed.filter(v => v.rule.includes(`\`${call}\``) && /no longer exists/.test(v.rule)).length, 1, `${call} renamed`)
    const unlocked = blockingViolations(`pub(crate) fn ${call}(&self) { forget(); }`, defined)
    assert.equal(unlocked.filter(v => v.rule.includes(`\`${call}\``) && /no longer takes/.test(v.rule)).length, 1, `${call} unlocked`)
    const intact = blockingViolations(`pub(crate) fn ${call}(&self) { ${via}; }`, defined)
    assert.deepEqual(intact.filter(v => v.rule.includes(`\`${call}\``)), [], `${call} intact`)
  }
})
