// MP-08 / MP-09 / MP-10 / MP-11: keep subprocess waits out of shared state paths.
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import { productionSource } from './lib/rust-production-source.mjs'

// Reviewed non-process receipt waits and domain output projections. Context
// changes require reviewing the receiver again; no whole file is exempt.
const domainCalls = JSON.parse(readFileSync(new URL('./kernel-domain-waits.json', import.meta.url), 'utf8'))
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
  return violations
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const files = execFileSync('rg', ['--files', 'apps/kernel/src'], { encoding: 'utf8' }).trim().split('\n').filter(path => path.endsWith('.rs'))
  const violations = files.flatMap(path => blockingViolations(readFileSync(path, 'utf8'), path))
  for (const violation of violations) console.error(`${violation.path}:${violation.line}: ${violation.rule}`)
  console.log(`Kernel blocking discipline: ${files.length} files checked, ${violations.length} violations`)
  process.exitCode = violations.length ? 1 : 0
}
