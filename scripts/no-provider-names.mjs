#!/usr/bin/env node
// OF-18: compute-provider business logic belongs to Cloud. OSS product code, deploy files and
// drill scripts stay provider-neutral: no provider names, server types, API hosts or pinned
// provider addresses. Test files and docs may use a provider name as sample data.
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const PROVIDER_PATTERNS = [
  ['provider name', /hetzner|hcloud\b|\bovh|boat\.dev|\bboat_|digitalocean|linode|vultr|scaleway|\b(?:aws|ec2|gcp|azure)\b/i],
  ['server type', /\b(?:cpx|ccx|cax)\d{2}\b|\bcx[2-5]2\b/i],
  ['provider API host', /amazonaws\.com|compute\.googleapis\.com|management\.azure\.com/i],
  ['pinned provider address', /195\.201\.123\.115/],
]

// Files that may name a provider: documentation, history and test files (sample data).
const EXEMPT_PATH = /^docs(?:-site)?\/|(?:^|\/)tests?\/|(?:^|\/)tests\.rs$|\.test(?:-support)?(?:\.[^/]*)?$|\.bun-test\.|(?:^|\/)pnpm-lock\.yaml$|(?:^|\/)Cargo\.lock$|\.md$/
const SELF = 'scripts/no-provider-names.mjs'

// Whole-file exceptions, each justified: the name is not a compute provider's business logic.
export const ALLOWED_FILES = new Map([
  ['apps/kernel/src/secret_redaction.rs', 'detects leaked cloud-account secret formats (AWS access keys); not compute-provider logic'],
  ['apps/kernel/src/runtime/state/claude_permission_rules.rs', 'denies reads of the well-known ~/.aws/credentials file; not compute-provider logic'],
  ['apps/cli/scripts/lib/managed-parity-semantic-reviews.mjs', 'historical MP-11 review record, pinned byte-for-byte to commit 9334141d4 by its own test; it names a blob that existed then'],
  ['apps/cli/scripts/lib/managed-parity-source-rules.mjs', 'historical MP-11 source rules paired with the pinned review record above; the ledger test requires matching paths'],
  ['apps/cli/scripts/benchmarks/qualification/guest-preflight.py', 'copies the OSWorld benchmark release table, whose "aws" key is upstream benchmark data'],
  ['apps/cli/scripts/fixtures/misbehaving-app/bundle/runtime/main.mjs', 'fixture app that prints a fake leaked secret to exercise redaction'],
])

// Rust unit tests live in the same file as the code; sample data there is fine.
function productText(path, text) {
  if (!path.endsWith('.rs')) return text
  const tests = text.search(/^#\[cfg\(test\)\]/m)
  return tests === -1 ? text : text.slice(0, tests)
}

export function providerNameFindings(path, text) {
  const findings = []
  for (const [kind, pattern] of PROVIDER_PATTERNS) {
    if (pattern.test(path)) findings.push({ file: path, line: 0, kind: `${kind} in path`, text: path })
  }
  productText(path, text).split('\n').forEach((line, index) => {
    for (const [kind, pattern] of PROVIDER_PATTERNS) {
      if (pattern.test(line)) findings.push({ file: path, line: index + 1, kind, text: line.trim().slice(0, 140) })
    }
  })
  return findings
}

export function isCheckedPath(path) {
  return path !== SELF && !ALLOWED_FILES.has(path) && !EXEMPT_PATH.test(path)
}

export function repositoryProviderNames(root, files = trackedFiles(root)) {
  return files.filter(isCheckedPath).flatMap(path => {
    let text
    try { text = readFileSync(resolve(root, path), 'utf8') } catch { return [] }
    if (text.includes('\u0000')) return []
    return providerNameFindings(path, text)
  })
}

function trackedFiles(root) {
  return execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8', maxBuffer: 1 << 28 })
    .split('\u0000').filter(Boolean)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const findings = repositoryProviderNames(resolve(fileURLToPath(import.meta.url), '../..'))
  for (const f of findings) console.error(`${f.file}:${f.line}: ${f.kind}: ${f.text}`)
  if (findings.length) {
    console.error(`\n${findings.length} provider reference(s) in OSS product code: provider logic belongs to Cloud (AGENTS.md "Runtime rules").`)
    process.exit(1)
  }
  console.log('no-provider-names: ok')
}
