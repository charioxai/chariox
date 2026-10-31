// OF-18: OSS product code stays free of compute-provider business logic.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { resolve } from 'node:path'
import { isCheckedPath, providerNameFindings, repositoryProviderNames } from './no-provider-names.mjs'

const kinds = (path, text) => providerNameFindings(path, text).map(f => f.kind)

test('flags provider names, server types, API hosts and pinned addresses with their line', () => {
  assert.deepEqual(kinds('a.ts', 'const MAX_HETZNER_VOLUME_SIZE_GB = 10'), ['provider name'])
  assert.deepEqual(kinds('a.ts', 'const id: "ovh" = 1'), ['provider name'])
  assert.deepEqual(kinds('a.ts', 'server_type: "cpx22"'), ['server type'])
  assert.deepEqual(kinds('a.ts', 'fetch("https://s3.us-east-1.amazonaws.com")'), ['provider API host'])
  assert.deepEqual(kinds('a.ts', 'wss://195.201.123.115.sslip.io'), ['pinned provider address'])
  assert.deepEqual(providerNameFindings('a.ts', 'ok\nhetzner').map(f => f.line), [2])
})

test('flags provider names in file paths', () => {
  assert.deepEqual(kinds('deploy/prepare-hetzner-image.sh', 'echo ok'), ['provider name in path'])
})

test('does not flag neutral words that contain a provider substring', () => {
  assert.deepEqual(kinds('a.ts', 'draws laws ctxval-cx03-local discovery cache'), [])
})

test('Rust unit tests, test files and docs may use provider names as sample data', () => {
  assert.deepEqual(kinds('a.rs', 'fn run() {}\n#[cfg(test)]\nmod tests { const ORIGIN: &str = "https://www.hetzner.com"; }'), [])
  assert.ok(!isCheckedPath('apps/cli/src/x.test.ts'))
  assert.ok(!isCheckedPath('apps/kernel/src/runtime/tests.rs'))
  assert.ok(!isCheckedPath('apps/kernel/src/local/api/tests/protocol_shapes/x.rs'))
  assert.ok(!isCheckedPath('docs/PROVIDER_ACCOUNTS.md'))
  assert.ok(isCheckedPath('deploy/managed-kernel/prepare-image.sh'))
})

test('the repository has no provider names in product code, deploy or drill scripts', () => {
  const root = resolve(fileURLToPath(import.meta.url), '../..')
  const findings = repositoryProviderNames(root).map(f => `${f.file}:${f.line}: ${f.text}`)
  assert.ok(findings.length === 0, `provider logic belongs to Cloud:\n${findings.slice(0, 40).join('\n')}`)
})
