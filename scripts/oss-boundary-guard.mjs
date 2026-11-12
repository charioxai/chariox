#!/usr/bin/env node
// OF-18+: compute-provider and Cloud-only business logic belongs to Cloud. OSS product code,
// deploy files and drill scripts stay neutral. Three checks:
//   1. vocabulary: provider names, server types, API hosts and pinned addresses (OF-18), plus
//      rental, pricing, billing, auto-stop, reimage, compute-class and vendor-host words;
//   2. control-plane API paths called from the kernel, CLI, kernel client and relay are listed
//      in scripts/control-plane-paths.allow.json;
//   3. Cloud-shaped local-API requests and kernel-client builders are listed in
//      scripts/control-plane-requests.allow.json.
// Test files and docs may use these words as sample data. The leak list
// (scripts/oss-boundary-leaks.allow.json) and the whole-file list (scripts/oss-boundary-files.allow.json)
// may only shrink: an entry whose file no longer matches fails, so deleting a leak means deleting
// its entry in the same change, and an entry that is not in the list at the merge base is red, so a
// swapped-in entry cannot hide behind a deleted one.
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const PROVIDER_PATTERNS = [
  ['provider name', /hetzner|hcloud\b|\bovh|boat\.dev|\bboat_|digitalocean|linode|vultr|scaleway|\b(?:aws|ec2|gcp|azure)\b/i],
  ['server type', /\b(?:cpx|ccx|cax)\d{2}\b|\bcx[2-5]2\b/i],
  ['provider API host', /amazonaws\.com|compute\.googleapis\.com|management\.azure\.com/i],
  ['pinned provider address', /195\.201\.123\.115/],
]

// Cloud business vocabulary: rental, pricing, billing and shutdown policy are decided by the
// connected service, never compiled into OSS. A word counts inside snake_case and SCREAMING_CASE
// identifiers (`hourly_price`) because `_` is not a word break here. `credits` and `region` stay allowed (relay flow
// control, screen regions).
const VOCABULARY_PATTERNS = [
  ['rental word', /(?<![A-Za-z0-9])rent(?:s|al|als|ed|ing)?(?![A-Za-z0-9])/i],
  ['pricing word', /(?<![A-Za-z0-9])pric(?:e|es|ing)(?![A-Za-z0-9])/i],
  ['billing word', /(?<![A-Za-z0-9])(?:billing|billable|invoices?)(?![A-Za-z0-9])/i],
  ['auto-stop word', /auto[-_ ]?stop/i],
  ['reimage word', /reimage|Reimage|REIMAGE/],
  ['compute class word', /compute[-_ ]?class/i],
  ['hourly word', /(?<![A-Za-z0-9])hourly(?![A-Za-z0-9])/i],
  ['vendor host', /(?:^|[^\w.-])(?:[a-z][a-z0-9-]*\.)*chariox\.com\b/i],
]

// Files that may name a provider or use the Cloud vocabulary: documentation, history, test files
// (sample data), spikes and vendored patches (upstream text).
const EXEMPT_PATH = /^(?:docs(?:-site)?|spikes|patches)\/|(?:^|\/)[Tt]ests?\/|(?:^|\/)tests\.rs$|_tests?\.rs$|\.test(?:-support)?(?:\.[^/]*)?$|\.bun-test\.|(?:^|\/)pnpm-lock\.yaml$|(?:^|\/)Cargo\.lock$|\.md$/
// The guard and its allowlists name the words they forbid.
const SELF_FILES = new Set(['scripts/oss-boundary-guard.mjs', 'scripts/oss-boundary-leaks.allow.json', 'scripts/oss-boundary-files.allow.json', 'scripts/control-plane-paths.allow.json', 'scripts/control-plane-requests.allow.json'])

// The one file that names the connected service a client signs in to by default.
export const DEFAULT_HOST_FILE = 'packages/kernel-client/src/default-control-plane.ts'

// Offset just past the Rust item that starts at `from`: its first `;` or closing `}` outside
// brackets, strings, chars and comments.
function itemEnd(text, from) {
  let depth = 0
  for (let i = from; i < text.length; i++) {
    const c = text[i]
    if (c === '/' && text[i + 1] === '/') { i = text.indexOf('\n', i); if (i === -1) return text.length; continue }
    if (c === '/' && text[i + 1] === '*') {
      let nesting = 1
      for (i += 2; i < text.length && nesting > 0; i++) {
        if (text[i] === '/' && text[i + 1] === '*') { nesting++; i++ } else if (text[i] === '*' && text[i + 1] === '/') { nesting--; i++ }
      }
      i--
      continue
    }
    const raw = /^b?r(#*)"/.exec(text.slice(i, i + 40))
    if (raw && !/[\w]/.test(text[i - 1] ?? '')) {
      const close = text.indexOf(`"${raw[1]}`, i + raw[0].length)
      if (close === -1) return text.length
      i = close + raw[1].length
      continue
    }
    if (c === '"') {
      for (i++; i < text.length && text[i] !== '"'; i++) if (text[i] === '\\') i++
      continue
    }
    if (c === "'") {
      if (text[i + 1] === '\\') i = text.indexOf("'", i + 3)
      else if (text[i + 2] === "'") i += 2
      if (i === -1) return text.length
      continue
    }
    if (c === '{' || c === '(' || c === '[') depth++
    else if (c === '}' || c === ')' || c === ']') {
      depth--
      if (depth === 0 && c === '}') return i + 1
    } else if (c === ';' && depth === 0) return i + 1
  }
  return text.length
}

// Rust unit tests live in the same file as the code and `#[cfg(test)]` covers only the item
// after it; blank each such item (keeping line numbers) and scan the production code around it.
function productText(path, text) {
  if (!path.endsWith('.rs')) return text
  let out = ''
  let cursor = 0
  const attribute = /^#\[cfg\(test\)\]/gm
  for (let hit = attribute.exec(text); hit; hit = attribute.exec(text)) {
    if (hit.index < cursor) continue
    const end = itemEnd(text, hit.index + hit[0].length)
    out += text.slice(cursor, hit.index) + text.slice(hit.index, end).replace(/[^\n]/g, '')
    cursor = end
  }
  return out + text.slice(cursor)
}

// `RentalQuote` -> `Rental Quote`, `parseHTTPServer` -> `parse HTTP Server`. The patterns are
// case-insensitive and cannot see a case change, so CamelCase text is also matched with a space at
// each lower->Upper and Acronym->Word boundary. Whole words only: a lower-case run is never split,
// so `current` or `parent` never yield `rent`. Embedded base64 data (a long mixed-case run) holds
// no identifiers and random case changes in it would spell banned words, so it is left alone.
const DATA_RUN = /[A-Za-z0-9+/=]{64,}/g
const isDataRun = run => /\d/.test(run) && /[a-z]/.test(run) && /[A-Z]/.test(run)
const splitWords = text => text
  .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
  .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')

export function splitIdentifierWords(text) {
  let out = ''
  let cursor = 0
  for (const run of text.matchAll(DATA_RUN)) {
    if (!isDataRun(run[0])) continue
    out += splitWords(text.slice(cursor, run.index)) + run[0]
    cursor = run.index + run[0].length
  }
  return out + splitWords(text.slice(cursor))
}

function matchedKinds(patterns, text) {
  const split = splitIdentifierWords(text)
  return patterns.filter(([, pattern]) => pattern.test(text) || pattern.test(split)).map(([kind]) => kind)
}

function patternFindings(path, text, patterns) {
  const findings = matchedKinds(patterns, path).map(kind => ({ file: path, line: 0, kind: `${kind} in path`, text: path }))
  productText(path, text).split('\n').forEach((line, index) => {
    for (const kind of matchedKinds(patterns, line)) findings.push({ file: path, line: index + 1, kind, text: line.trim().slice(0, 140) })
  })
  return findings
}

// deploy/managed-kernel installs a kernel on any machine a control plane or an operator provisions,
// so its scripts name the update coordinator by role, never "Cloud" (OSB-06). `cloud-init` is the
// Linux provisioning tool and stays allowed.
export const NEUTRAL_DEPLOY_ROOT = 'deploy/managed-kernel/'
const NEUTRAL_DEPLOY_PATTERNS = [['connected-service word', /(?<![A-Za-z0-9])cloud(?![A-Za-z0-9]|-init)/i]]

export const providerNameFindings = (path, text) => patternFindings(path, text, PROVIDER_PATTERNS)
export const vocabularyFindings = (path, text) => patternFindings(path, text,
  path.startsWith(NEUTRAL_DEPLOY_ROOT) ? [...VOCABULARY_PATTERNS, ...NEUTRAL_DEPLOY_PATTERNS] : VOCABULARY_PATTERNS)

// Product code for the path and request checks; the whole-file list only excuses names and vocabulary.
const isProductPath = path => !SELF_FILES.has(path) && !EXEMPT_PATH.test(path)
export const isCheckedPath = (path, allowedFiles = new Set()) => isProductPath(path) && !allowedFiles.has(path)

const PATH_SCAN_ROOTS = [
  'apps/kernel/src/', 'apps/cli/src/', 'apps/cli/scripts/', 'packages/kernel-client/src/', 'apps/relay/src/',
  'apps/server/src/', 'apps/setup/', 'deploy/', 'docker/', 'packages/aegs-sdk/src/', 'scripts/',
]
const PATH_SCAN_EXTENSION = /\.(?:rs|tsx?|mjs|cjs|js|sh)$|(?:^|\/)Dockerfile[^/]*$/
// A string literal that starts with a control-plane path, optionally after a `{}`, `${..}` or `$NAME` base.
const PATH_LITERAL = /["'`](?:\{\}|\$\{[^}]*\}|\$\w+)?(\/(?:v1\/|managed-|control-plane|relay\/|kernels\/|browser\/)[^"'`\s]*)/g

// Control-plane API paths in a source file, normalized: no query, placeholders as `{}`.
export function controlPlanePaths(path, text) {
  const found = []
  productText(path, text).split('\n').forEach((line, index) => {
    const code = line.trimStart()
    if (code.startsWith('//') || code.startsWith('*') || code.startsWith('/*') || code.startsWith('#')) return
    for (const hit of line.matchAll(PATH_LITERAL)) {
      const normalized = hit[1].split('?')[0].replace(/\$\{[^}]*(?:\}|$)/g, '{}').replace(/\{[^}]*\}/g, '{}').replace(/\\+$/, '')
      if (/\.[a-z]+$/.test(normalized)) continue // a file name such as /managed-vault.json, not an API path
      found.push({ path: normalized, line: index + 1 })
    }
  })
  return found
}

// Variant names of the Rust enum `name`, written one per line at four-space indent.
export function enumVariants(text, name) {
  const start = text.indexOf(`pub enum ${name} {`)
  if (start === -1) return []
  const body = text.slice(start).split('\n').slice(1)
  const end = body.findIndex(line => line.startsWith('}'))
  return body.slice(0, end === -1 ? undefined : end).flatMap(line => /^ {4}([A-Z]\w*)/.exec(line)?.[1] ?? [])
}

const CLOUD_SHAPED_REQUEST = /Cloud|Managed|Disposable|Reimage|AutoStop|Rent|Billing|Hosted|Price|Image/
const REQUEST_ENUMS = [
  ['apps/kernel/src/local/api/types/request.rs', 'LocalDaemonRequest'],
  ['apps/kernel/src/local/api/types/control_plane.rs', 'ControlPlaneRequest'],
]
const CLIENT_BUILDER_FILE = /^packages\/kernel-client\/src\/ipc-(?:managed|disposable|cloud)-[^/]*\.ts$/
const REQUEST_REASON = /^(?:generic seam: |Cloud-only, deleted by ).{8,}/

function readText(root, path) {
  try {
    const text = readFileSync(resolve(root, path), 'utf8')
    return text.includes('\u0000') ? null : text
  } catch { return null }
}

function readAllowlist(root, name) {
  const path = resolve(root, 'scripts', name)
  return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')) : []
}

const problem = (file, kind, text, line = 0) => ({ file, line, kind, text })

// Every leak is owned: the plan row it belongs to and the slice (docs/design/OSS_BOUNDARY_PLAN.md,
// section 2) that deletes it.
export const LEAK_ROWS = new Set(['OSB-04', 'OSB-05', 'OSB-06', 'OSB-07', 'OSB-09'])
export const LEAK_SLICES = new Set(['B0', 'B1', 'B2', 'B3', 'B4a', 'B4b', 'B5', 'B6', 'B7', 'B8'])

// The two shrink-only lists. Whole-file exceptions each carry a reason: the name is not a compute
// provider's business logic. Leak entries each name the item that deletes them. An entry that is
// not in the list at the merge base fails, and it excuses nothing. The accepted list is read from
// git history, not from the working tree, so editing the list cannot excuse the entry that was
// just added to it.
const ALLOWED_FILES_LIST = { label: 'whole-file list', file: 'scripts/oss-boundary-files.allow.json', key: 'path', newKind: 'new allowed file' }
const LEAK_LIST = { label: 'leak list', file: 'scripts/oss-boundary-leaks.allow.json', key: 'path', newKind: 'new leak entry' }
const SHRINK_ONLY_LISTS = [ALLOWED_FILES_LIST, LEAK_LIST]
const gitOut = (root, ...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 1 << 28 }).trim()

// The commit the change branches from: CHARIOX_BOUNDARY_BASE, else the merge base of HEAD and
// origin/main. `reason` says why there is none. An explicit base that is not a commit throws.
function baselineCommit(root, env) {
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

// For each shrink-only list, the entry keys at the accepted baseline. In CI (CI=true) a missing
// baseline throws: a runner that cannot name one would let every entry pass. Locally `keys` is
// null, with a note saying what was not checked, when there is no baseline to compare with.
export function acceptedEntries(root, env = process.env) {
  const { base, reason } = baselineCommit(root, env)
  if (!base && env.CI === 'true') {
    throw new Error(`${reason}. CI must fetch the commit the change branches from and export CHARIOX_BOUNDARY_BASE=<sha> for this step (see the "Fetch the commit the change branches from" step in .github/workflows/ci.yml); without it every list entry would pass unchecked`)
  }
  return new Map(SHRINK_ONLY_LISTS.map(list => {
    const unchecked = why => ({ keys: null, note: `${list.label} ${list.file}: ${why}; membership NOT checked, so an entry added in this change is not detected` })
    if (!base) return [list, unchecked(reason)]
    let text
    try { text = gitOut(root, 'show', `${base}:./${list.file}`) } catch {
      return [list, unchecked(`not in ${base.slice(0, 12)}, so the list is introduced by this change`)]
    }
    return [list, { keys: new Set(JSON.parse(text).map(entry => entry[list.key])), note: null }]
  }))
}

const isAccepted = (list, entry, accepted) => !accepted.get(list).keys || accepted.get(list).keys.has(entry[list.key])

function newEntryProblems(list, entries, accepted) {
  return entries.filter(entry => entry[list.key] && !isAccepted(list, entry, accepted))
    .map(entry => problem(list.file, list.newKind, `${entry[list.key]} is not in the list at the merge base: move the logic to Cloud instead of adding an entry`))
}

function allowedFileProblems(root, files, entries) {
  const problems = []
  const tracked = new Set(files)
  for (const entry of entries) {
    const text = entry.path && tracked.has(entry.path) ? readText(root, entry.path) : null
    if (!entry.path || !entry.why) problems.push(problem(ALLOWED_FILES_LIST.file, 'allowed file entry', `${entry.path ?? '?'} needs a path and a why`))
    else if (text === null || providerNameFindings(entry.path, text).length + vocabularyFindings(entry.path, text).length === 0) {
      problems.push(problem(entry.path, 'stale allowed file', `no longer names a provider or Cloud word: delete the entry from ${ALLOWED_FILES_LIST.file}`))
    }
  }
  return problems
}

function vocabularyProblems(root, files, accepted) {
  const leaks = new Map()
  const problems = []
  const entries = readAllowlist(root, 'oss-boundary-leaks.allow.json')
  problems.push(...newEntryProblems(LEAK_LIST, entries, accepted))
  // An entry that is not in the accepted list excuses nothing: its file is scanned and reported too.
  for (const entry of entries.filter(entry => isAccepted(LEAK_LIST, entry, accepted))) {
    if (!entry.path || !entry.removedBy) { leaks.set(entry.path ?? '?', null); continue }
    if (!LEAK_ROWS.has(entry.row) || !LEAK_SLICES.has(entry.slice)) {
      problems.push(problem(entry.path, 'leak entry', `needs an owner row (${[...LEAK_ROWS].join('|')}) and a plan slice (${[...LEAK_SLICES].join('|')})`))
    }
    leaks.set(entry.path, entry)
  }
  const allowedEntries = readAllowlist(root, 'oss-boundary-files.allow.json')
  problems.push(...newEntryProblems(ALLOWED_FILES_LIST, allowedEntries, accepted), ...allowedFileProblems(root, files, allowedEntries))
  const allowedFiles = new Set(allowedEntries.filter(entry => entry.why && isAccepted(ALLOWED_FILES_LIST, entry, accepted)).map(entry => entry.path))
  const matched = new Set()
  for (const path of files.filter(path => isCheckedPath(path, allowedFiles))) {
    const text = readText(root, path)
    if (text === null) continue
    problems.push(...providerNameFindings(path, text))
    const vocabulary = vocabularyFindings(path, text).filter(f => !(path === DEFAULT_HOST_FILE && f.kind === 'vendor host'))
    if (vocabulary.length === 0) continue
    if (leaks.has(path)) matched.add(path)
    else problems.push(...vocabulary.map(f => ({ ...f, kind: `${f.kind}: Cloud-only vocabulary in OSS (no entry in scripts/oss-boundary-leaks.allow.json)` })))
  }
  for (const [path, entry] of leaks) {
    if (entry === null) problems.push(problem(path, 'leak entry', 'needs a path and the PR or item that removes it (removedBy)'))
    else if (!matched.has(path)) problems.push(problem(path, 'stale leak entry', `no longer matches: delete the entry (${entry.removedBy})`))
  }
  return problems
}

function controlPlanePathProblems(root, files) {
  const entries = readAllowlist(root, 'control-plane-paths.allow.json')
  const known = new Map()
  const problems = []
  for (const entry of entries) {
    const label = entry.path ?? '?'
    // A path nobody has classified yet names the slice that decides it instead of a seam.
    const owned = entry.seam || LEAK_SLICES.has(entry.slice)
    if (!entry.path || !entry.why || !owned || !Array.isArray(entry.callers) || entry.callers.length === 0) {
      problems.push(problem('scripts/control-plane-paths.allow.json', 'path entry', `${label} needs path, callers, why and a seam or a plan slice`))
      continue
    }
    known.set(entry.path, entry)
  }
  const seen = new Map()
  for (const path of files.filter(f => PATH_SCAN_ROOTS.some(r => f.startsWith(r)) && PATH_SCAN_EXTENSION.test(f) && isProductPath(f))) {
    const text = readText(root, path)
    if (text === null) continue
    for (const found of controlPlanePaths(path, text)) {
      const entry = known.get(found.path)
      if (!entry) problems.push(problem(path, 'unlisted control-plane path', `${found.path} is not in scripts/control-plane-paths.allow.json`, found.line))
      else if (!entry.callers.includes(path)) problems.push(problem(path, 'unlisted control-plane caller', `${found.path} is called here but its entry names ${entry.callers.join(', ')}`, found.line))
      else seen.set(`${found.path} ${path}`, true)
    }
  }
  for (const entry of known.values()) {
    for (const caller of entry.callers) {
      if (!seen.has(`${entry.path} ${caller}`)) problems.push(problem('scripts/control-plane-paths.allow.json', 'stale control-plane path', `${entry.path} is no longer called from ${caller}: delete it from the entry`))
    }
  }
  return problems
}

function requestProblems(root, files) {
  const entries = readAllowlist(root, 'control-plane-requests.allow.json')
  const present = new Set()
  for (const [path, name] of REQUEST_ENUMS) {
    const text = readText(root, path)
    if (text !== null) enumVariants(text, name).filter(v => CLOUD_SHAPED_REQUEST.test(v)).forEach(v => present.add(`${name}::${v}`))
  }
  files.filter(f => CLIENT_BUILDER_FILE.test(f) && isProductPath(f)).forEach(f => present.add(f))
  const problems = []
  const listed = new Map(entries.map(entry => [entry.name, entry]))
  for (const name of present) {
    const entry = listed.get(name)
    if (!entry) problems.push(problem(name.includes('::') ? 'scripts/control-plane-requests.allow.json' : name, 'unlisted Cloud-shaped request', `${name} is not in scripts/control-plane-requests.allow.json`))
    else if (!REQUEST_REASON.test(entry.why ?? '')) problems.push(problem('scripts/control-plane-requests.allow.json', 'request entry', `${name} needs a why that starts with "generic seam: " or "Cloud-only, deleted by "`))
  }
  for (const name of listed.keys()) {
    if (!present.has(name)) problems.push(problem('scripts/control-plane-requests.allow.json', 'stale request entry', `${name} no longer exists: delete the entry`))
  }
  return problems
}

export function boundaryProblems(root, files = trackedFiles(root), accepted = acceptedEntries(root)) {
  return [...vocabularyProblems(root, files, accepted), ...controlPlanePathProblems(root, files), ...requestProblems(root, files)]
}

function trackedFiles(root) {
  return execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8', maxBuffer: 1 << 28 })
    .split('\u0000').filter(Boolean)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(fileURLToPath(import.meta.url), '../..')
  let accepted
  try { accepted = acceptedEntries(root) } catch (error) {
    console.error(`oss-boundary-guard: ${error.message}`)
    process.exit(1)
  }
  for (const { note } of accepted.values()) if (note) console.error(`oss-boundary-guard: WARNING ${note}`)
  const problems = boundaryProblems(root, undefined, accepted)
  for (const f of problems) console.error(`${f.file}:${f.line}: ${f.kind}: ${f.text}`)
  if (problems.length) {
    console.error(`\n${problems.length} OSS boundary problem(s): provider and Cloud-only logic belongs to Cloud (AGENTS.md "Runtime rules"); allowlists may only shrink.`)
    process.exit(1)
  }
  console.log('oss-boundary-guard: ok')
}
