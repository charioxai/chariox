import { closing, inside, lineOf } from './rust-scan.mjs'

const processResources = 'apps/kernel/src/runtime/process_resources.rs'
const processControl = 'apps/kernel/src/runtime/process_resources/control.rs'
const handover = 'apps/kernel/src/runtime/process_resources/handover.rs'
const projection = 'apps/kernel/src/runtime/process_resources/projection.rs'
const launch = 'apps/kernel/src/runtime/process_resources/launch.rs'
const providerProcesses = 'apps/kernel/src/app/provider_processes.rs'
const ptyManager = 'apps/kernel/src/pty/manager.rs'

// Entry points that take the process-resources registry mutex, directly (`via: 'registry()'`) or through the entry
// point or private helper named in `via`. `form` is how a caller names it: a method (`x.call(..)`), a path
// (`process_resources::call(..)`) or a plain function. Whatever ends, lists or hands over a process waits for that
// mutex, so a runtime thread that takes it waits behind them. Add the entry point here, not its callers, when a new
// function takes the registry lock; the lint checks every row against its definition. A method name other types
// share (`release`, `observe`) names the `receiver` the production callers use, so the others are not flagged.
export const registryLockCalls = [
  { call: 'bind_key', form: 'method', defined: processResources, via: 'registry()' },
  { call: 'unbind_key', form: 'method', defined: processResources, via: 'registry()' },
  { call: 'has_key', form: 'path', defined: processResources, via: 'registry()' },
  { call: 'samples_for_key', form: 'path', defined: processResources, via: 'registry()' },
  { call: 'submit_line_for_key', form: 'path', defined: processResources, via: 'observation_for_key(' },
  { call: 'outside_ended_for_key', form: 'path', defined: processControl, via: 'registry()' },
  { call: 'stop_for_key', form: 'path', defined: processControl, via: 'ask(' },
  { call: 'start_now_for_key', form: 'path', defined: processControl, via: 'ask(' },
  { call: 'end_owner', form: 'path', defined: processControl, via: 'registry()' },
  { call: 'stop_released', form: 'path', defined: processControl, via: 'released_keys(' },
  { call: 'keys_for_session', form: 'path', defined: projection, via: 'listed_keys(' },
  { call: 'for_session', form: 'path', defined: projection, via: 'observed_keys_for_session(' },
  { call: 'hand_over_released', form: 'path', defined: handover, via: 'registry()' },
  { call: 'adopt_released', form: 'path', defined: handover, via: 'start_released(' },
  { call: 'bind_terminal_for_key', form: 'path', defined: processResources, via: 'with_submissions_for_key(' },
  { call: 'line_started_for_key', form: 'path', defined: processResources, via: 'with_submissions_for_key(' },
  { call: 'line_finished_for_key', form: 'path', defined: processResources, via: 'with_submissions_for_key(' },
  { call: 'release', form: 'method', receiver: 'resources', defined: processResources, via: 'registry()' },
  { call: 'observe', form: 'method', receiver: 'permit', defined: launch, via: 'samples_for_key(' },
  { call: 'release_exited_process_jobs', form: 'method', defined: ptyManager, via: 'release(' },
  { call: 'remove_process', form: 'method', defined: ptyManager, via: 'unbind_key(' },
  { call: 'remove_provider_pty_process', form: 'function', defined: providerProcesses, via: 'remove_process(' },
  { call: 'remove_run', form: 'method', defined: providerProcesses, via: 'remove_provider_pty_process(' },
]

const callPattern = ({ call, form, receiver = '' }) => new RegExp({
  method: `${receiver && `\\b${receiver}`}\\.${call}\\s*\\(`,
  path: `\\bprocess_resources::${call}\\s*\\(`,
  function: `(?<![.\\w:])(?<!\\bfn\\s+)${call}\\s*\\(`,
}[form], 'g')

const spansOf = (code, pattern) => [...code.matchAll(pattern)]
  .map(match => [match.index + match[0].length - 1, closing(code, match.index + match[0].length - 1)])
  .filter(([, end]) => end > 0)

// Code that runs on a runtime (async) thread: async fn bodies, async blocks, and the closure given to
// `with_app_side_effect`, which runs inline under the async app lock. The closure given to `spawn_blocking`,
// `block_in_place`, `with_app_side_effect_blocking` or a new thread runs on a blocking thread, where the wait is
// safe. The innermost span wins.
function runtimeSpans(code) {
  const runtime = [
    ...spansOf(code, /\basync\s+fn\s+\w+\b[^{;]*\{/g),
    ...spansOf(code, /\basync\s+(?:move\s+)?\{/g),
    ...spansOf(code, /\bwith_app_side_effect\s*\(/g),
  ].map(span => ({ span, runtime: true }))
  const blocking = spansOf(code, /\b(?:spawn_blocking|block_in_place|with_app_side_effect_blocking|thread::spawn)\s*\(/g)
    .map(span => ({ span, runtime: false }))
  return [...runtime, ...blocking]
}

function enclosingFn(code, at) {
  let name = '<none>', width = Infinity
  for (const match of code.matchAll(/\bfn\s+(\w+)\b[^{;]*\{/g)) {
    const open = match.index + match[0].length - 1, end = closing(code, open)
    if (open < at && at < end && end - open < width) { name = match[1]; width = end - open }
  }
  return name
}

// A call to a registry lock entry point on a runtime thread must be a reviewed entry:
// `{ path, fn, call, reason }` (kernel-registry-lock-calls.json), where `fn` is the function that makes the call.
export function registryLockOnRuntime(code, path, allowed) {
  const spans = runtimeSpans(code), hits = []
  for (const entry of registryLockCalls) {
    for (const match of code.matchAll(callPattern(entry))) {
      const innermost = spans.filter(({ span: [a, b] }) => match.index > a && match.index < b)
        .sort((x, y) => (x.span[1] - x.span[0]) - (y.span[1] - y.span[0]))[0]
      if (!innermost?.runtime) continue
      const fn = enclosingFn(code, match.index)
      if (allowed.some(site => site.path === path && site.fn === fn && site.call === entry.call)) continue
      hits.push({ path, line: lineOf(code, match.index), rule: `registry lock: \`${entry.call}(..)\` takes the process-resources registry mutex on a runtime thread in \`${fn}\`: use spawn_blocking / with_app_side_effect_blocking, or review it into kernel-registry-lock-calls.json` })
    }
  }
  return hits
}

// The table rows defined in `path` must still exist and still reach the registry lock; otherwise the table is stale
// and the callers would pass silently.
export function registryLockTable(code, path) {
  const hits = []
  for (const entry of registryLockCalls.filter(row => row.defined === path)) {
    const bodies = [...code.matchAll(new RegExp(`\\bfn\\s+${entry.call}\\b[^{;]*\\{`, 'g'))]
      .map(match => code.slice(match.index + match[0].length - 1, closing(code, match.index + match[0].length - 1)))
    if (!bodies.length) hits.push({ path, line: 1, rule: `registry lock entry point \`${entry.call}\` named in lib/kernel-registry-lock-calls.mjs no longer exists: update the table` })
    else if (bodies.some(body => !body.includes(entry.via))) hits.push({ path, line: 1, rule: `registry lock entry point \`${entry.call}\` no longer takes \`${entry.via}\`: update lib/kernel-registry-lock-calls.mjs` })
  }
  return hits
}
