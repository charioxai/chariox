#!/usr/bin/env node
// Runs every rule check in scripts/checks/*.mjs (pnpm lint:rules). A check is {name, rule, rootFix, ratchet, scan(ctx)} and returns
// findings {file, line, what}; this file applies the shrink-only list scripts/checks/<name>.allow.json (scripts/lib/ratchet.mjs) and
// prints one line per failure: file:line: what is wrong. Root fix: ... Rule: ...
//   node scripts/checks/run.mjs [--root DIR] [--only a,b] [--shrink | --init]
// --shrink lowers counts and deletes entries that no longer match. It never adds an entry or raises a count.
// --init writes the list of a new check from today's findings; it refuses when the list file already exists.
import { execFileSync } from 'node:child_process'
import { existsSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { acceptedEntries, countedProblems, gitOut, readList, readText, requiredBaseline, trackedFiles } from '../lib/ratchet.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const args = process.argv.slice(2)
const flag = name => args.includes(`--${name}`)
const option = name => (args.includes(`--${name}`) ? args[args.indexOf(`--${name}`) + 1] : undefined)

// What a check may look at: tracked files, their text, a line search, and the diff against the merge base.
function context(root, baseline) {
  const files = trackedFiles(root)
  const cache = new Map()
  return {
    root,
    files,
    read: path => { if (!cache.has(path)) cache.set(path, readText(root, path)); return cache.get(path) },
    size: path => { try { return statSync(resolve(root, path)).size } catch { return 0 } },
    // [{file, line, text}] for every line of a tracked text file matching the extended regex (git grep -E).
    grep(pattern, { paths = [], ignoreCase = false } = {}) {
      let out
      try {
        out = execFileSync('git', ['grep', '-n', '-I', '-z', '--no-color', ...(ignoreCase ? ['-i'] : []), '-E', '-e', pattern, '--', ...(paths.length ? paths : ['.'])], { cwd: root, encoding: 'utf8', maxBuffer: 1 << 29 })
      } catch (error) { if (error.status === 1) return []; throw error }
      const hits = []
      for (const record of out.split('\n').filter(Boolean)) {
        const [file, line, ...text] = record.split('\u0000')
        hits.push({ file, line: Number(line), text: text.join('').replace(/^\n/, '') })
      }
      return hits
    },
    // The commit the change branches from, or null with the reason; a diff check cannot judge a change without it.
    base: baseline.base ?? null,
    baseReason: baseline.reason ?? null,
    // The file's text at the merge base, or null when the file did not exist there or there is no base.
    readAtBase(path) {
      if (!baseline.base) return null
      try { return gitOut(root, 'show', `${baseline.base}:${path}`) } catch { return null }
    },
    // Added and removed lines per file against the base, working tree included: Map file -> [{sign, line, text}].
    diff() {
      if (!baseline.base) return new Map()
      const out = gitOut(root, 'diff', '--no-color', '--unified=0', '--no-renames', baseline.base, '--')
      const changes = new Map()
      let rows = null
      let line = 0
      for (const row of out.split('\n')) {
        if (row.startsWith('diff --git ')) { const file = row.slice(row.lastIndexOf(' b/') + 3); rows = []; changes.set(file, rows) } else if (row.startsWith('@@')) line = Number(/\+(\d+)/.exec(row)[1])
        else if (!rows || row.startsWith('+++') || row.startsWith('---')) continue
        else if (row.startsWith('+')) rows.push({ sign: '+', line: line++, text: row.slice(1) })
        else if (row.startsWith('-')) rows.push({ sign: '-', line, text: row.slice(1) })
      }
      return changes
    },
  }
}

async function loadChecks(only) {
  const names = readdirSync(here).filter(f => f.endsWith('.mjs') && !f.endsWith('.test.mjs') && f !== 'run.mjs').sort()
  const checks = []
  for (const file of names) {
    const check = (await import(pathToFileURL(resolve(here, file)).href)).default
    if (!only || only.includes(check.name)) checks.push(check)
  }
  return checks
}

// One entry per line keeps a shrinking diff to the lines that went.
const writeList = (root, file, entries) => writeFileSync(resolve(root, file), entries.length ? `[\n${entries.map(e => ` ${JSON.stringify(e)}`).join(',\n')}\n]\n` : '[]\n')

// One object per check: the accepted-entries map is keyed by it.
const lists = new Map()
const listOf = check => {
  if (!lists.has(check.name)) lists.set(check.name, { label: `${check.name} list`, file: `scripts/checks/${check.name}.allow.json`, key: 'path', newKind: 'new entry' })
  return lists.get(check.name)
}

async function main() {
  const root = resolve(option('root') ?? resolve(here, '../..'))
  const only = option('only')?.split(',')
  const checks = await loadChecks(only)
  const env = process.env
  const ratcheted = checks.filter(check => check.ratchet !== 'diff')
  let accepted
  let baseline
  try {
    baseline = requiredBaseline(root, env)
    accepted = acceptedEntries(root, ratcheted.map(listOf), baseline)
  } catch (error) { console.error(`lint:rules: ${error.message}`); process.exit(1) }
  const unchecked = [...accepted].filter(([, { note }]) => note)
  if (unchecked.length) console.error(`lint:rules: WARNING ${unchecked.map(([list]) => list.label).join(', ')}: ${unchecked[0][1].note.replace(/^.*?: /, '')}`)
  const ctx = context(root, baseline)
  const lines = []
  const timings = []
  for (const check of checks) {
    const started = performance.now()
    if (check.ratchet === 'diff' && !ctx.base) console.error(`lint:rules: WARNING ${check.name} compared nothing: ${ctx.baseReason}`)
    let findings
    try { findings = check.scan(ctx) } catch (error) { console.error(`lint:rules: ${check.name}: ${error.message}`); process.exit(1) }
    if (check.ratchet === 'diff') lines.push(...findings.map(f => `${f.file}:${f.line ?? 0}: ${f.what}. Root fix: ${check.rootFix}. Rule: ${check.rule}`))
    else {
      const list = listOf(check)
      const entries = readList(root, list.file)
      const counts = new Map()
      for (const f of findings) counts.set(f.file, (counts.get(f.file) ?? 0) + 1)
      if (flag('shrink')) {
        writeList(root, list.file, entries.flatMap(e => (counts.has(e.path) ? [{ ...e, count: Math.min(e.count, counts.get(e.path)) }] : [])))
        continue
      }
      if (flag('init')) {
        if (existsSync(resolve(root, list.file))) throw new Error(`${list.file} exists: a list only shrinks (--shrink)`)
        writeList(root, list.file, [...counts].map(([path, count]) => ({ path, count, why: check.why })))
        continue
      }
      lines.push(...countedProblems({ check, list, findings, entries, accepted }))
    }
    timings.push(`${check.name} ${Math.round(performance.now() - started)} ms`)
  }
  if (flag('shrink') || flag('init')) return
  for (const line of lines) console.error(line)
  if (lines.length) {
    console.error(`\nlint:rules: ${lines.length} problem(s). Each line names the root fix; the lists in scripts/checks/*.allow.json only shrink (scripts/checks/README.md).`)
    process.exit(1)
  }
  console.log(`lint:rules: ok (${timings.join(', ')})`)
}

await main()
