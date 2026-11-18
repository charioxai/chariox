// Shrink-only ("ratchet") lists, shared by scripts/oss-boundary-guard.mjs and scripts/checks/*.
// A list holds today's violations. It may only shrink: an entry that is not in the list at the merge base is red (a swapped-in
// entry cannot hide behind a deleted one), an entry whose file no longer matches is red, and a count that fell must be lowered.
// The accepted list is read from git history, not from the working tree, so editing the list cannot excuse the entry just added.
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'

export const gitOut = (root, ...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 1 << 28 }).trim()

export function trackedFiles(root) {
  return execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8', maxBuffer: 1 << 28 }).split('\u0000').filter(Boolean)
}

export function readText(root, path) {
  try {
    const text = readFileSync(resolve(root, path), 'utf8')
    return text.includes('\u0000') ? null : text
  } catch { return null }
}

export function readList(root, file) {
  const path = resolve(root, file)
  return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')) : []
}

export const problem = (file, kind, text, line = 0) => ({ file, line, kind, text })

// The commit the change branches from: CHARIOX_BOUNDARY_BASE, else the merge base of HEAD and
// origin/main. `reason` says why there is none. An explicit base that is not a commit throws.
export function baselineCommit(root, env) {
  try { gitOut(root, 'rev-parse', '--git-dir') } catch { return { reason: 'not a git repository' } }
  if (env.CHARIOX_BOUNDARY_BASE) {
    try { return { base: gitOut(root, 'rev-parse', '--verify', `${env.CHARIOX_BOUNDARY_BASE}^{commit}`) } } catch {
      throw new Error(`CHARIOX_BOUNDARY_BASE=${env.CHARIOX_BOUNDARY_BASE} is not a commit in ${root}: unset it or name the commit the change branches from`)
    }
  }
  try { return { base: gitOut(root, 'merge-base', 'HEAD', 'origin/main') } } catch {
    return { reason: 'no origin/main to compare with (fetch it or set CHARIOX_BOUNDARY_BASE to the commit the change branches from)' }
  }
}

// In CI (CI=true) a missing baseline throws: a runner that cannot name one would let every entry pass.
export function requiredBaseline(root, env) {
  const found = baselineCommit(root, env)
  if (!found.base && env.CI === 'true') {
    throw new Error(`${found.reason}. CI must fetch the commit the change branches from and export CHARIOX_BOUNDARY_BASE=<sha> for this step (see the "Fetch the commit the change branches from" step in .github/workflows/ci.yml); without a baseline every entry would pass`)
  }
  return found
}

// For each list ({label, file, key, newKind}), the entries at the accepted baseline. Locally `keys` is null, with a note saying
// what was not checked, when there is no baseline to compare with.
export function acceptedEntries(root, lists, env = process.env) {
  const { base, reason } = requiredBaseline(root, env)
  return new Map(lists.map(list => {
    const unchecked = why => ({ keys: null, entries: null, note: `${list.label} ${list.file}: ${why}; membership NOT checked, so an entry added in this change is not detected` })
    if (!base) return [list, unchecked(reason)]
    let text
    try { text = gitOut(root, 'show', `${base}:./${list.file}`) } catch {
      return [list, unchecked(`not in ${base.slice(0, 12)}, so the list is introduced by this change`)]
    }
    const entries = new Map(JSON.parse(text).map(entry => [entry[list.key], entry]))
    return [list, { keys: new Set(entries.keys()), entries, note: null }]
  }))
}

export const isAccepted = (list, entry, accepted) => !accepted.get(list).keys || accepted.get(list).keys.has(entry[list.key])

export function newEntryProblems(list, entries, accepted, advice) {
  return entries.filter(entry => entry[list.key] && !isAccepted(list, entry, accepted))
    .map(entry => problem(list.file, list.newKind, `${entry[list.key]} is not in the list at the merge base: ${advice}`))
}

// One line per finding: where, what is wrong, the root fix, the rule it comes from.
export const failureLine = (finding, check) => `${finding.file}:${finding.line ?? 0}: ${finding.what}. Root fix: ${check.rootFix}. Rule: ${check.rule}`

// Counted ratchet: findings are {file, line, what}; list entries are {path, count, why}. A file may hold at most as many
// findings as the accepted entry says, and the count may only fall. Returns the failure lines.
export function countedProblems({ check, list, findings, entries, accepted }) {
  const lines = []
  const byFile = new Map()
  for (const f of findings) byFile.set(f.file, [...(byFile.get(f.file) ?? []), f])
  const allowed = new Map()
  const shrink = 'node scripts/checks/run.mjs --shrink does it'
  for (const entry of entries) {
    if (!entry.path || !Number.isInteger(entry.count) || entry.count < 1 || !entry.why) {
      lines.push(`${list.file}:0: ${entry.path ?? '?'} needs a path, a count of at least 1 and a why`)
      continue
    }
    const base = accepted.get(list).entries?.get(entry.path)
    if (accepted.get(list).keys && !base) continue // a new entry is reported below and excuses nothing
    if (base && entry.count > base.count) {
      lines.push(`${list.file}:0: ${entry.path} allows ${entry.count} but the merge base allows ${base.count}. Root fix: ${check.rootFix}. Rule: ${check.rule}`)
      allowed.set(entry.path, base.count)
    } else allowed.set(entry.path, entry.count)
    const found = byFile.get(entry.path)?.length ?? 0
    if (found === 0) lines.push(`${entry.path}:0: stale entry, the file no longer matches: delete it from ${list.file} (${shrink})`)
    else if (found < entry.count) lines.push(`${entry.path}:0: ${found} left, the list says ${entry.count}: lower it in ${list.file} (${shrink})`)
  }
  for (const p of newEntryProblems(list, entries, accepted, `${check.rootFix} instead of adding an entry`)) lines.push(`${p.file}:0: ${p.kind}: ${p.text}`)
  for (const [file, found] of byFile) {
    if (found.length > (allowed.get(file) ?? 0)) for (const f of found) lines.push(failureLine(f, check))
  }
  return lines
}
