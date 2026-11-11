// MP-08 / MP-09 / MP-10 / MP-11: keep subprocess waits and lock chains out of shared state paths.
//
// Rules:
//  1. raw subprocess waits and the environment lock (kernel-domain-waits.json lists the reviewed receipt waits);
//  2. a std/parking_lot Mutex/RwLock guard bound with `let` that is still live at an `.await` of its block
//     (kernel-lock-awaits.json lists the reviewed ones: { path, guard, init, reason });
//  3. an `.await` that suspends a long-lived scheduler or pump loop itself without a bound
//     (kernel-loop-waits.json lists the reviewed ones: { path, fn, call, reason }). A freeze of such a loop
//     stops every timer, wake or output it serves, so its slow work runs timed out or as a single-flight task.
//  4. a call to a process-resources registry lock entry point (lib/kernel-registry-lock-calls.mjs lists them) from a
//     runtime thread: an async body, or the closure of `with_app_side_effect`, which runs inline under the app lock.
//     The registry mutex is held by whatever ends, lists or hands over a process, so the runtime thread waits behind
//     it (the healthnext r2 stack: remove_provider_pty_process -> unbind_key). Use `spawn_blocking` /
//     `with_app_side_effect_blocking`, or review the site into kernel-registry-lock-calls.json
//     ({ path, fn, call, reason }); the rows of the entry point table are checked against their definitions.
//     The session-store lock is not covered: it is taken inline by design at ~200 production sites.
//
// Long-lived loops checked by rule 3 (add a loop here when it serves every room):
//  | file                                      | function                       | serves                            |
//  |-------------------------------------------|--------------------------------|-----------------------------------|
//  | runtime/state/agent_wake_scheduler.rs     | run_agent_wake_scheduler       | agent timers, wakes, deadlines    |
//  | runtime/state/agent_wake_scheduler.rs     | sweep_agent_wakes              | 30 s dead-man sweep of the wakes  |
//  | runtime/state/agent_wake_scheduler.rs     | tick_agent_wakes               | due wake firing (async, no await) |
//  | runtime/state/app_wake_pump_runtime.rs    | run_app_wake_scheduler         | app wake delivery                 |
//  | runtime/state/provider_output_pump_runtime.rs | run_provider_output_pump_until | provider output into history  |
//  | runtime/state/disk_pressure.rs            | run_disk_maintenance           | disk pressure scans (own task)    |
import { readdirSync, readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import { lockAcrossAwait } from './lib/kernel-lock-across-await.mjs'
import { loopWaits } from './lib/kernel-loop-waits.mjs'
import { registryLockOnRuntime, registryLockTable } from './lib/kernel-registry-lock-calls.mjs'
import { productionSource } from './lib/rust-production-source.mjs'

// Reviewed non-process receipt waits and domain output projections. Context
// changes require reviewing the receiver again; no whole file is exempt.
const domainCalls = JSON.parse(readFileSync(new URL('./kernel-domain-waits.json', import.meta.url), 'utf8'))
const lockAwaits = JSON.parse(readFileSync(new URL('./kernel-lock-awaits.json', import.meta.url), 'utf8'))
const loopAwaits = JSON.parse(readFileSync(new URL('./kernel-loop-waits.json', import.meta.url), 'utf8'))
const registryAwaits = JSON.parse(readFileSync(new URL('./kernel-registry-lock-calls.json', import.meta.url), 'utf8'))
const loopFunctions = {
  'apps/kernel/src/runtime/state/agent_wake_scheduler.rs': ['run_agent_wake_scheduler', 'tick_agent_wakes', 'sweep_agent_wakes'],
  'apps/kernel/src/runtime/state/app_wake_pump_runtime.rs': ['run_app_wake_scheduler'],
  'apps/kernel/src/runtime/state/provider_output_pump_runtime.rs': ['run_provider_output_pump_until'],
  'apps/kernel/src/runtime/state/disk_pressure.rs': ['run_disk_maintenance'],
}
function domainCall(code, path, match) {
  const context = code.slice(Math.max(0, match.index - 3000), match.index).replace(/\s+/g, ' ').trim().split(' ').slice(-60).join(' ')
  return domainCalls.some(site => site.path === path && site.method === match[1] && site.context === context)
}

export function blockingViolations(source, path) {
  if (path.includes('/runtime/bounded_process') || path.endsWith('/env_lock.rs')) return []
  const code = productionSource(source, path), violations = []
  const add = (at, rule) => violations.push({ path, line: code.slice(0, at).split('\n').length, rule })
  for (const match of code.matchAll(/\.(wait|output|wait_with_output)\s*\(\s*\)/g)) {
    if (!domainCall(code, path, match)) add(match.index, 'raw subprocess call: use bounded_process')
  }
  for (const match of code.matchAll(/\b(?:Child::(?:wait|wait_with_output)|Command::(?:output|status))\s*\(/g)) add(match.index, 'raw subprocess UFCS: use bounded_process')
  for (const match of code.matchAll(/\benv_lock::(?:lock|EnvGuard)\b/g)) add(match.index, 'production environment lock')
  if (path.endsWith('/provider/service/store.rs')) {
    for (const match of code.matchAll(/self\.write\(\)\s*\.\s*(?:start_run_provider_only|prepare_run_provider_only)\(/g)) add(match.index, 'provider connect under STORE')
  }
  violations.push(...lockAcrossAwait(code, path, lockAwaits))
  violations.push(...registryLockOnRuntime(code, path, registryAwaits), ...registryLockTable(code, path))
  if (loopFunctions[path]) violations.push(...loopWaits(code, path, loopFunctions[path], loopAwaits))
  return violations
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const files = readdirSync('apps/kernel/src', { recursive: true }).filter(path => path.endsWith('.rs')).map(path => `apps/kernel/src/${path}`).sort()
  const violations = files.flatMap(path => blockingViolations(readFileSync(path, 'utf8'), path))
  for (const violation of violations) console.error(`${violation.path}:${violation.line}: ${violation.rule}`)
  console.log(`Kernel blocking discipline: ${files.length} files checked, ${violations.length} violations`)
  process.exitCode = violations.length ? 1 : 0
}
