// OF-18+: OSS product code stays free of compute-provider and Cloud-only business logic.
import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { dirname, join, resolve } from 'node:path'
import { boundaryProblems, controlPlanePaths, enumVariants, isCheckedPath, providerNameFindings, vocabularyFindings } from './oss-boundary-guard.mjs'

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

test('Rust production code after a test-only item is still scanned', () => {
  assert.deepEqual(kinds('a.rs', '#[cfg(test)]\nuse crate::hetzner_fixture;\nconst LIMIT: &str = "hetzner";'), ['provider name'])
  assert.deepEqual(kinds('a.rs', '#[cfg(test)]\nmod tests { const A: &str = "}hetzner{"; fn f() { /* } */ } }\nfn run() { let s = "ovh"; }'), ['provider name'])
  assert.deepEqual(kinds('a.rs', '#[cfg(test)]\n#[allow(dead_code)]\nfn helper() { let c = \'{\'; let r = r#"{ hetzner"#; }\nfn tail() {}'), [])
  assert.deepEqual(providerNameFindings('a.rs', 'mod tests {}\n#[cfg(test)]\nmod t { fn x() { "ovh" } }\nfn run() {}\n#[cfg(test)]\nmod u {}\nfn late() { "vultr" }').map(f => f.line), [7])
})

test('flags Cloud business vocabulary, and not neutral words that contain it', () => {
  const words = text => vocabularyFindings('a.ts', text).map(f => f.kind)
  assert.deepEqual(words('const preset = "idle_15m"; // auto-stop policy'), ['auto-stop word'])
  assert.deepEqual(words('managedAutoStopPreset: "idle_15m"'), ['auto-stop word'])
  assert.deepEqual(words('rent a machine; show the price per hour'), ['rental word', 'pricing word'])
  assert.deepEqual(words('Billing and invoices; hourly; compute class'), ['billing word', 'compute class word', 'hourly word'])
  assert.deepEqual(words('reimageManagedEnvironment("e1")'), ['reimage word'])
  assert.deepEqual(words('fetch("https://staging.chariox.com/relay/token")'), ['vendor host'])
  assert.deepEqual(words('current parent transparent requireLegacyRestoreImage chariox.commands.search credits region'), [])
  assert.deepEqual(vocabularyFindings('apps/cli/src/waiting-room-reimage.ts', '').map(f => f.kind), ['reimage word in path'])
})

test('control-plane path literals are normalized and exclude file names and test code', () => {
  const found = (path, text) => controlPlanePaths(path, text).map(f => f.path)
  assert.deepEqual(found('a.ts', 'get(`/managed-environments/${encodeURIComponent(id)}/lifecycle?x=1`)'), ['/managed-environments/{}/lifecycle'])
  assert.deepEqual(found('a.rs', 'format!("{}/v1/subscriptions/reconcile", url); let p = "/v1/managed-kernels/activity";'), ['/v1/subscriptions/reconcile', '/v1/managed-kernels/activity'])
  assert.deepEqual(found('a.rs', 'const V: &str = "/managed-vault.json";\n// "/v1/x"\n#[cfg(test)]\nmod tests { const P: &str = "/v1/y"; }'), [])
  assert.deepEqual(found('a.ts', 'const u = "/auth/device/start"'), [])
})

test('enum variants are read at four-space indent', () => {
  const text = 'pub enum Request {\n    #[serde(rename = "x")]\n    Alpha(AlphaRequest),\n    Beta {\n        field: u8,\n    },\n}\npub enum Other {\n    Gamma,\n}'
  assert.deepEqual(enumVariants(text, 'Request'), ['Alpha', 'Beta'])
})

// A scratch repository with the three allowlists; each test plants one leak in it.
function scratch(files, allow = {}) {
  const root = mkdtempSync(join(tmpdir(), 'oss-boundary-'))
  const write = (path, text) => { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), text) }
  for (const [path, text] of Object.entries(files)) write(path, text)
  write('scripts/oss-boundary-leaks.allow.json', JSON.stringify(allow.leaks ?? []))
  write('scripts/control-plane-paths.allow.json', JSON.stringify(allow.paths ?? []))
  write('scripts/control-plane-requests.allow.json', JSON.stringify(allow.requests ?? []))
  return { root, files: Object.keys(files), done: () => rmSync(root, { recursive: true, force: true }) }
}
const problemsOf = (files, allow) => {
  const repo = scratch(files, allow)
  try { return boundaryProblems(repo.root, repo.files).map(p => `${p.kind}: ${p.file}`) } finally { repo.done() }
}

test('a planted Cloud vocabulary leak is red until it is listed, and a stale entry is red', () => {
  const files = { 'apps/cli/src/plan.ts': 'export const preset = "agents_done" // auto-stop policy\n' }
  assert.equal(problemsOf(files).length, 1)
  assert.match(problemsOf(files)[0], /^auto-stop word: Cloud-only vocabulary in OSS.*: apps\/cli\/src\/plan\.ts$/)
  assert.deepEqual(problemsOf(files, { leaks: [{ path: 'apps/cli/src/plan.ts', removedBy: 'item 2' }] }), [])
  assert.deepEqual(problemsOf({ 'apps/cli/src/plan.ts': 'export const preset = 1\n' }, { leaks: [{ path: 'apps/cli/src/plan.ts', removedBy: 'item 2' }] }), ['stale leak entry: apps/cli/src/plan.ts'])
  assert.deepEqual(problemsOf(files, { leaks: [{ path: 'apps/cli/src/plan.ts' }] }), ['leak entry: apps/cli/src/plan.ts'])
  assert.deepEqual(problemsOf({ 'apps/cli/src/plan.test.ts': 'const rent = "auto-stop"' }), [])
})

test('a vendor host is allowed only in the default-host file', () => {
  const host = 'export const DEFAULT_CONTROL_PLANE_URL = "https://chariox.com"\n'
  assert.deepEqual(problemsOf({ 'packages/kernel-client/src/default-control-plane.ts': host }), [])
  assert.equal(problemsOf({ 'apps/cli/src/other.ts': host }).length, 1)
  assert.equal(problemsOf({ 'packages/kernel-client/src/default-control-plane.ts': 'const x = "reimage"\n' }).length, 1)
})

test('a planted control-plane path is red until it is listed with a caller, and a stale caller is red', () => {
  const files = { 'apps/kernel/src/runtime/new_client.rs': 'const P: &str = "/v1/machines/start";\n' }
  const entry = { path: '/v1/machines/start', callers: ['apps/kernel/src/runtime/new_client.rs'], why: 'w', seam: 's' }
  assert.deepEqual(problemsOf(files), ['unlisted control-plane path: apps/kernel/src/runtime/new_client.rs'])
  assert.deepEqual(problemsOf(files, { paths: [entry] }), [])
  assert.deepEqual(problemsOf(files, { paths: [{ ...entry, callers: [] }] }).sort(), ['path entry: scripts/control-plane-paths.allow.json', 'unlisted control-plane path: apps/kernel/src/runtime/new_client.rs'].sort())
  assert.deepEqual(problemsOf(files, { paths: [{ ...entry, callers: ['apps/kernel/src/runtime/new_client.rs', 'apps/cli/src/gone.ts'] }] }), ['stale control-plane path: scripts/control-plane-paths.allow.json'])
  assert.deepEqual(problemsOf({ ...files, 'apps/cli/src/other.ts': 'get("/v1/machines/start")' }, { paths: [entry] }), ['unlisted control-plane caller: apps/cli/src/other.ts'])
})

test('a planted Cloud-shaped request or kernel-client builder is red until it is justified', () => {
  const files = {
    'apps/kernel/src/local/api/types/request.rs': 'pub enum LocalDaemonRequest {\n    ListSlices(A),\n    CloudMachine(B),\n}\n',
    'packages/kernel-client/src/ipc-cloud-ledger-requests.ts': 'export {}\n',
  }
  assert.deepEqual(problemsOf(files).sort(), ['unlisted Cloud-shaped request: packages/kernel-client/src/ipc-cloud-ledger-requests.ts', 'unlisted Cloud-shaped request: scripts/control-plane-requests.allow.json'].sort())
  const requests = [
    { name: 'LocalDaemonRequest::CloudMachine', why: 'generic seam: opaque class ids passed to the control plane' },
    { name: 'packages/kernel-client/src/ipc-cloud-ledger-requests.ts', why: 'Cloud-only, deleted by OSS-boundary item 99' },
  ]
  assert.deepEqual(problemsOf(files, { requests }), [])
  assert.deepEqual(problemsOf(files, { requests: [{ ...requests[0], why: 'needed' }, requests[1]] }), ['request entry: scripts/control-plane-requests.allow.json'])
  assert.deepEqual(problemsOf({ ...files, 'packages/kernel-client/src/ipc-cloud-ledger-requests.ts': 'export {}\n' }, { requests: [...requests, { name: 'LocalDaemonRequest::Gone', why: 'generic seam: removed long ago' }] }), ['stale request entry: scripts/control-plane-requests.allow.json'])
})

test('the repository has no unlisted provider names, Cloud vocabulary, control-plane paths or Cloud-shaped requests', () => {
  const root = resolve(fileURLToPath(import.meta.url), '../..')
  const findings = boundaryProblems(root).map(f => `${f.file}:${f.line}: ${f.kind}: ${f.text}`)
  assert.ok(findings.length === 0, `provider and Cloud-only logic belongs to Cloud:\n${findings.slice(0, 40).join('\n')}`)
})
