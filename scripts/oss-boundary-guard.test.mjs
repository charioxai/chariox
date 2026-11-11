// OF-18+: OSS product code stays free of compute-provider and Cloud-only business logic.
import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { dirname, join, resolve } from 'node:path'
import * as guard from './oss-boundary-guard.mjs'
import { boundaryProblems, controlPlanePaths, enumVariants, isCheckedPath, providerNameFindings, vocabularyFindings } from './oss-boundary-guard.mjs'

const boundaryFiles = root => execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8' }).split('\u0000').filter(Boolean)
const isProductSource = path => /\.(?:ts|mjs|js)$/.test(path) && !/\.test(?:-support)?\.|\.bun-test\./.test(path)
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
  assert.deepEqual(words('const HOURLY_PRICE_EUR: f64 = 0.0116; let rent_machine = 1;'), ['rental word', 'pricing word', 'hourly word'])
  assert.deepEqual(words('reimageManagedEnvironment("e1")'), ['reimage word'])
  assert.deepEqual(words('fetch("https://staging.chariox.com/relay/token")'), ['vendor host'])
  assert.deepEqual(words('current parent transparent requireLegacyRestoreImage chariox.commands.search credits region'), [])
  assert.deepEqual(vocabularyFindings('apps/cli/src/waiting-room-reimage.ts', '').map(f => f.kind), ['reimage word in path'])
})

test('CamelCase identifiers built from banned words are caught, whole words only', () => {
  const all = text => [...providerNameFindings('a.ts', text), ...vocabularyFindings('a.ts', text)].map(f => f.kind)
  assert.deepEqual(guard.splitIdentifierWords('RentalQuote hetznerServer parseHTTPServer cpx21Server'), 'Rental Quote hetzner Server parse HTTP Server cpx21 Server')
  assert.deepEqual(all('class RentalQuote {}'), ['rental word'])
  assert.deepEqual(all('type HetznerServer = 1'), ['provider name'])
  assert.deepEqual(all('const hetznerServer = 1'), ['provider name'])
  assert.deepEqual(all('rentManagedMachine()'), ['rental word'])
  assert.deepEqual(all('interface AutoStopPolicy {}'), ['auto-stop word'])
  assert.deepEqual(all('getOvhHost(); new AwsClient()'), ['provider name'])
  assert.deepEqual(all('const quotePriceEur = 1; HourlyRate'), ['pricing word', 'hourly word'])
  assert.deepEqual(vocabularyFindings('apps/cli/src/rentalQuote.ts', '').map(f => f.kind), ['rental word in path'])
  // Words that merely contain a banned word, and names the repository already uses, stay clean.
  assert.deepEqual(all('currentParent transparentPricey authorizedRenter drawLaws boatLoad'), [])
  assert.deepEqual(all(`const bytes = Buffer.from("${'AGFzbQEAAAAB7AROY'.repeat(4)}+Aws${'CQCAAKAIE9'.repeat(4)}")`), [])
  assert.deepEqual(all('const baseUrl = urlFor(Parent); applyRegion(Credits); AllowedHostPattern'), [])
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
const fullProblemsOf = (files, allow) => {
  const repo = scratch(files, allow)
  try { return boundaryProblems(repo.root, repo.files) } finally { repo.done() }
}
const problemsOf = (files, allow) => fullProblemsOf(files, allow).map(p => `${p.kind}: ${p.file}`)

test('a planted Cloud vocabulary leak is red until it is listed, and a stale entry is red', () => {
  const files = { 'apps/cli/src/plan.ts': 'export const preset = "agents_done" // auto-stop policy\n' }
  assert.equal(problemsOf(files).length, 1)
  assert.match(problemsOf(files)[0], /^auto-stop word: Cloud-only vocabulary in OSS.*: apps\/cli\/src\/plan\.ts$/)
  assert.deepEqual(problemsOf(files, { leaks: [{ path: 'apps/cli/src/plan.ts', removedBy: 'item 2', row: 'OSB-09', slice: 'B2' }] }), [])
  assert.deepEqual(problemsOf({ 'apps/cli/src/plan.ts': 'export const preset = 1\n' }, { leaks: [{ path: 'apps/cli/src/plan.ts', removedBy: 'item 2', row: 'OSB-09', slice: 'B2' }] }), ['stale leak entry: apps/cli/src/plan.ts'])
  assert.deepEqual(problemsOf(files, { leaks: [{ path: 'apps/cli/src/plan.ts', row: 'OSB-09', slice: 'B2' }] }), ['leak entry: apps/cli/src/plan.ts'])
  assert.deepEqual(problemsOf({ 'apps/cli/src/plan.test.ts': 'const rent = "auto-stop"' }), [])
})

test('deploy/managed-kernel names the update coordinator by role, never "Cloud" (OSB-06)', () => {
  const words = text => vocabularyFindings('deploy/managed-kernel/upgrade-image.sh', text).map(f => f.kind)
  assert.deepEqual(words('require_cloud_update_target "$root"'), ['connected-service word'])
  assert.deepEqual(words('console.error("Cloud managed release update target")'), ['connected-service word'])
  assert.deepEqual(words('# a cloud-init user-data file'), [])
  assert.deepEqual(vocabularyFindings('apps/cli/src/x.ts', 'CloudRelayStatus cloud').map(f => f.kind), [])
})

test('OSB-07: the hosted trigger ingress in apps/server names no connected-service route', () => {
  const root = resolve(fileURLToPath(import.meta.url), '../..')
  const files = boundaryFiles(root).filter(f => f.startsWith('apps/server/src/') && isProductSource(f))
  // An empty or truncated file list would pass the check below without scanning anything.
  assert.ok(files.length >= 10 && files.includes('apps/server/src/agent-trigger-admission.ts'), `OSB-07 scanned ${files.length} files: ${files.slice(0, 3).join(', ')}`)
  const scan = (list, read) => list.flatMap(f => controlPlanePaths(f, read(f)).map(c => `${f}:${c.line}: ${c.path}`))
  assert.deepEqual(scan(['apps/server/src/planted.ts'], () => 'await fetch(`${base}/v1/managed-kernels/activity`)'), ['apps/server/src/planted.ts:1: /v1/managed-kernels/activity'], 'the scan reports a planted route')
  const calls = scan(files, f => readFileSync(join(root, f), 'utf8'))
  assert.ok(calls.length === 0, `apps/server calls a control-plane route; configure an admission URL instead:\n${calls.join('\n')}`)
  const listed = JSON.parse(readFileSync(join(root, 'scripts/control-plane-paths.allow.json'), 'utf8')).filter(e => e.callers.some(c => c.startsWith('apps/server/')))
  assert.ok(listed.length === 0, `scripts/control-plane-paths.allow.json still lists apps/server callers: ${listed.map(e => e.path).join(', ')}`)
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

test('a control-plane path in apps/server or under /browser/ is seen by the guard', () => {
  const planted = { 'apps/server/src/hosted-trigger.ts': 'const u = new URL(`/browser/x/${id}/admission`, origin)\n' }
  assert.deepEqual(problemsOf(planted), ['unlisted control-plane path: apps/server/src/hosted-trigger.ts'])
  const entry = { path: '/browser/x/{}/admission', callers: ['apps/server/src/hosted-trigger.ts'], why: 'w', seam: 'generic seam: hosted trigger admission' }
  assert.deepEqual(problemsOf(planted, { paths: [entry] }), [])
  assert.deepEqual(problemsOf({ 'packages/aegs-sdk/src/server.rs': 'const P: &str = "/v1/subscriptions";\n' }), ['unlisted control-plane path: packages/aegs-sdk/src/server.rs'])
  assert.deepEqual(problemsOf({ 'scripts/e2e-stack/live.mjs': 'await get("/relay/token")\n' }), ['unlisted control-plane path: scripts/e2e-stack/live.mjs'])
  assert.deepEqual(problemsOf({ 'deploy/run.sh': 'curl "$ORIGIN/v1/owners/kernels"\n' }), ['unlisted control-plane path: deploy/run.sh'])
})

test('a leak-list entry needs an owner row and a plan slice', () => {
  const files = { 'apps/cli/src/plan.ts': 'export const preset = "agents_done" // auto-stop policy\n' }
  const entry = { path: 'apps/cli/src/plan.ts', removedBy: 'B2', row: 'OSB-09', slice: 'B2' }
  assert.deepEqual(problemsOf(files, { leaks: [entry] }), [])
  for (const broken of [{ row: undefined }, { slice: undefined }, { row: 'OSB-99' }, { slice: 'B9' }, { row: 'reimage' }]) {
    assert.deepEqual(problemsOf(files, { leaks: [{ ...entry, ...broken }] }), ['leak entry: apps/cli/src/plan.ts'], JSON.stringify(broken))
  }
  assert.match(fullProblemsOf(files, { leaks: [{ ...entry, row: undefined }] })[0].text, /owner row/)
})

test('the leak list may not grow past the recorded total', () => {
  assert.ok(Number.isInteger(guard.LEAK_ENTRY_CEILING) && guard.LEAK_ENTRY_CEILING > 0, 'the guard records the leak-list ceiling')
  const files = {}
  const leaks = []
  for (let i = 0; i <= guard.LEAK_ENTRY_CEILING; i++) {
    files[`apps/cli/src/leak${i}.ts`] = 'export const preset = "agents_done" // auto-stop policy\n'
    leaks.push({ path: `apps/cli/src/leak${i}.ts`, removedBy: 'B2', row: 'OSB-09', slice: 'B2' })
  }
  assert.deepEqual(problemsOf(files, { leaks }), ['leak list grew: scripts/oss-boundary-leaks.allow.json'])
  assert.deepEqual(problemsOf(Object.fromEntries(Object.entries(files).slice(1)), { leaks: leaks.slice(1) }), [])
})

test('the recorded leak ceiling is the current entry count, so deleting entries lowers it', () => {
  const entries = JSON.parse(readFileSync(resolve(fileURLToPath(import.meta.url), '../oss-boundary-leaks.allow.json'), 'utf8'))
  assert.ok(entries.length === guard.LEAK_ENTRY_CEILING, `${entries.length} entries but the ceiling is ${guard.LEAK_ENTRY_CEILING}: set LEAK_ENTRY_CEILING to the new total`)
})

test('the repository has no unlisted provider names, Cloud vocabulary, control-plane paths or Cloud-shaped requests', () => {
  const root = resolve(fileURLToPath(import.meta.url), '../..')
  const findings = boundaryProblems(root).map(f => `${f.file}:${f.line}: ${f.kind}: ${f.text}`)
  assert.ok(findings.length === 0, `provider and Cloud-only logic belongs to Cloud:\n${findings.slice(0, 40).join('\n')}`)
})

// The `pnpm lint` slot runs this script first: a throwaway commit that adds a Cloud-only request
// variant, provider vocabulary and an unlisted control-plane path turns it red, and dropping that
// commit turns it green again. Each plant is named in the output so a reviewer sees which rule it broke.
test('a planted-violation commit turns the guard CLI red and removing it turns it green', () => {
  const root = mkdtempSync(join(tmpdir(), 'oss-boundary-drill-'))
  const git = (...args) => { const r = spawnSync('git', ['-c', 'user.name=drill', '-c', 'user.email=drill@example.test', ...args], { cwd: root, encoding: 'utf8' }); assert.equal(r.status, 0, r.stderr); return r.stdout }
  const write = (path, text) => { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), text) }
  const guard = () => spawnSync(process.execPath, ['scripts/oss-boundary-guard.mjs'], { cwd: root, encoding: 'utf8' })
  try {
    git('init', '-q', '-b', 'main')
    mkdirSync(join(root, 'scripts'))
    copyFileSync(fileURLToPath(new URL('./oss-boundary-guard.mjs', import.meta.url)), join(root, 'scripts/oss-boundary-guard.mjs'))
    for (const name of ['oss-boundary-leaks', 'control-plane-paths', 'control-plane-requests']) write(`scripts/${name}.allow.json`, '[]')
    write('apps/kernel/src/local/api/types/request.rs', 'pub enum LocalDaemonRequest {\n    ListSlices(A),\n}\n')
    git('add', '-A'); git('commit', '-q', '-m', 'clean')
    const clean = guard()
    assert.equal(clean.status, 0, clean.stderr)
    write('apps/kernel/src/local/api/types/request.rs', 'pub enum LocalDaemonRequest {\n    ListSlices(A),\n    RentManagedMachine(B),\n}\n')
    write('apps/kernel/src/runtime/rental_quote.rs', 'const SERVER_TYPE: &str = "cpx21";\nconst QUOTE_PATH: &str = "/v1/rentals/quote";\n// A Hetzner host with an auto-stop deadline.\n')
    write('apps/cli/src/camel-plants.ts', 'export interface RentalQuote {}\nexport type HetznerServer = number\nexport interface AutoStopPolicy {}\nexport const rentManagedMachine = () => 1\n')
    git('add', '-A'); git('commit', '-q', '-m', 'planted violation')
    const red = guard()
    assert.equal(red.status, 1)
    for (const named of ['server type', 'provider name', 'auto-stop word', 'unlisted control-plane path', 'unlisted Cloud-shaped request: LocalDaemonRequest::RentManagedMachine']) {
      assert.ok(red.stderr.includes(named), `missing "${named}" in:\n${red.stderr}`)
    }
    for (const identifier of ['RentalQuote', 'HetznerServer', 'AutoStopPolicy', 'rentManagedMachine']) {
      assert.ok(red.stderr.split('\n').some(line => line.startsWith('apps/cli/src/camel-plants.ts:') && line.includes(identifier)), `CamelCase ${identifier} not reported in:\n${red.stderr}`)
    }
    git('reset', '-q', '--hard', 'HEAD~1')
    const green = guard()
    assert.equal(green.status, 0, green.stderr)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
