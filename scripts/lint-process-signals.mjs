#!/usr/bin/env node
// MP-11: class enforcement for repository script/test/drill group signalling.
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export function signalViolations(source) {
  const sites = []
  const rules = [
    ['node-group', /process\s*(?:\.\s*kill|\[\s*['"]kill['"]\s*\])\s*\(\s*([^,)]+)(?:,\s*([^)]*))?\)/g,
      match => /-|\b(?:group\w*|\w*(?:[Gg]roup|[Pp]gid)\w*)\b/.test(match[1])],
    ['node-alias', /\b(?:\w+\s*=\s*process\s*\.\s*kill|process\s*\.\s*kill\s*[,}])/g],
    ['node-owned-numeric', /\b(?:signalOwnedProcess(?:Group)?|ownedProcessGroupExists)\s*\(\s*((?:[A-Za-z_$][\w$]*(?:\s*\.\s*[A-Za-z_$][\w$]*)*\s*\.\s*pid)\b|(?:pid|pgid|groupId|processGroupId|group)\b|[-+]?\d+\b)/g],
    ['python-owned-numeric', /\b(?:owned_signals|owned)\s*\.\s*(?:group|pid)\s*\(\s*(?:[\w.]+\.pid\b|(?:pid|pgid|browser_pid)\b|os\.(?:getpid|getpgrp)\s*\(|[-+]?\d+\b)/g],
    ['python-group', /\bos\s*\.\s*killpg\b/g],
    ['name-signal', /\b(?:pkill|killall)\b/g],
    ['shell-group', /\bkill\s+(?:-s\s+\S+|-(?:SIG)?[A-Z0-9]+)\s+(?:--\s+)?["'\\]*-(?:\$|[0-9])/g],
  ]
  for (const [kind, pattern, predicate] of rules) {
    for (const match of source.matchAll(pattern)) {
      if (predicate && !predicate(match)) continue
      const line = source.slice(0, match.index).split('\n').length
      const text = source.split('\n')[line - 1].trim()
      sites.push({ kind, line, text })
    }
  }
  return sites.sort((a, b) => a.line - b.line || a.kind.localeCompare(b.kind))
}

export function lintProcessSignals({ root, files, allowlist = { sites: [], guards: [] } }) {
  const failures = [], allowed = [], used = new Map(), guardsUsed = new Set()
  let count = 0
  for (const path of files) {
    if (!/\.(?:[cm]?js|tsx?|py|sh|bash)$/.test(path)) continue
    const source = readFileSync(resolve(root, path), 'utf8')
    const sites = signalViolations(source)
    count += sites.length
    const guard = allowlist.guards.find(entry => entry.path === path)
    if (guard) {
      guardsUsed.add(guard.path)
      if (!guard.reason?.trim() || guard.sha256 !== createHash('sha256').update(source).digest('hex')) {
        failures.push({ path, line: 1, kind: 'guard-review-drift', text: 'MP-11 guard source requires review/hash update' })
      } else allowed.push(...sites.map(site => ({ path, ...site, reason: guard.reason })))
      continue
    }
    for (const site of sites) {
      const key = `${path}\0${site.kind}\0${site.text}`
      const entry = allowlist.sites.find(entry => entry.path === path && entry.kind === site.kind && entry.text === site.text)
      if (!entry?.reason?.trim()) failures.push({ path, ...site })
      else { used.set(key, (used.get(key) ?? 0) + 1); allowed.push({ path, ...site, reason: entry.reason }) }
    }
  }
  for (const entry of allowlist.sites) {
    if (used.get(`${entry.path}\0${entry.kind}\0${entry.text}`) !== entry.count) {
      failures.push({ path: entry.path, line: 1, kind: 'allowlist-drift', text: 'MP-11 reviewed site absent or occurrence count changed' })
    }
  }
  for (const guard of allowlist.guards) if (!guardsUsed.has(guard.path)) {
    failures.push({ path: guard.path, line: 1, kind: 'guard-review-drift', text: 'MP-11 reviewed guard absent' })
  }
  return { mpItem: 'MP-11', count, allowedCount: allowed.length, violationCount: failures.length, failures, allowed }
}

export function repositorySignalLint(root = resolve(import.meta.dirname, '..')) {
  const files = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'], { cwd: root, encoding: 'utf8' }).split('\0').filter(Boolean)
  const allowlist = JSON.parse(readFileSync(resolve(root, 'scripts/process-signals-allowlist.json'), 'utf8'))
  return lintProcessSignals({ root, files, allowlist })
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const result = repositorySignalLint()
  console.log(JSON.stringify(result, null, 2))
  process.exitCode = result.violationCount ? 1 : 0
}
