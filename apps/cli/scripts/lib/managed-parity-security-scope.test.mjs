// MP-11 classifier and narrowed admission contract.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { classifySecurityPath, SECURITY_CLASSES, evaluateParityMatrix, evaluateSecurityGate, B211_PARITY_ITEMS } from './managed-parity-security-scope.mjs';

const paths = {
  credentials: 'apps/kernel/src/secret/vault.rs',
  relay: 'apps/relay/src/auth.rs',
  signing: 'scripts/sign-managed-release.mjs',
  sandbox: 'apps/app-worker/src/sandbox_linux.c',
  apps: 'packages/app-package/src/developer/keys.rs',
  access: 'apps/kernel/src/runtime/kernel_access/policy.rs',
  signals: 'apps/kernel/src/process_spawn.rs',
  browser: 'apps/kernel/slice-linux-docker/docker/browser-controller-snapshot.mjs',
};
for (const name of Object.keys(SECURITY_CLASSES)) test(`MP-11 ${name}: positive, ordinary negative, tests excluded`, () => {
  assert.ok(classifySecurityPath(paths[name]).includes(name));
  assert.ok(!classifySecurityPath('apps/cli/src/theme.ts', 'const accent = "blue";').includes(name));
  assert.deepEqual(classifySecurityPath(paths[name].replace(/\.[^.]+$/, '.test.mjs')), []);
});

test('MP-11 content catches misplaced signal, capture, signature and credential operations', () => {
  for (const [text, name] of [['process.kill(pid, "SIGTERM")', 'signals'], ['Page.createIsolatedWorld', 'browser'], ['verify_strict(signature)', 'signing'], ['CLAUDE_CONFIG_DIR', 'credentials']]) {
    assert.ok(classifySecurityPath('scripts/new-helper.mjs', text).includes(name));
  }
});
test('MP-11 unknown classes inside trust roots fail closed', () => {
  assert.deepEqual(classifySecurityPath('apps/app-worker/src/future_boundary.c'), ['unknown']);
  assert.equal(evaluateSecurityGate({ entries: [{ securityClass: 'unknown', semanticDisposition: { status: 'reviewed' } }], parity: { status: 'pass' } }).status, 'fail');
});
const source = { commit: 'a'.repeat(40), tree: 'b'.repeat(40) };
const matrix = () => ({ schema: 'chariox.mp11.behavioural-parity.v1', sourceCommit: source.commit, sourceTree: source.tree,
  rows: [...Array.from({ length: 10 }, (_, i) => `MP-${String(i + 1).padStart(2, '0')}`), ...B211_PARITY_ITEMS].map(id => ({ id, status: 'GREEN', evidence: 'external/current-evidence' })) });
const entry = () => ({ securityClass: 'credentials', semanticDisposition: { status: 'reviewed', disposition: 'ordinary_path1_behavior' } });
const gate = overrides => evaluateSecurityGate({ entries: [entry()], parity: evaluateParityMatrix(matrix(), source), ...overrides });

test('MP-11 gate requires BOTH exact-source parity and current critical reviews', () => {
  assert.equal(gate({}).status, 'pass');
  assert.equal(gate({ parity: evaluateParityMatrix(null, source) }).status, 'fail');
  assert.equal(gate({ entries: [{ ...entry(), semanticDisposition: { status: 'unreviewed' } }] }).status, 'fail');
  assert.equal(gate({ entries: [{ ...entry(), semanticDisposition: { status: 'reviewed', disposition: 'removal_required' } }] }).status, 'fail');
  assert.equal(gate({ entries: [] }).status, 'fail');
  assert.equal(gate({ staleReviews: 1 }).status, 'fail');
  assert.equal(gate({ gaps: [{}] }).status, 'fail');
});
test('MP-11 parity accepts owner-visible RED disposition and rejects missing, duplicate, stale or unresolved rows', () => {
  const red = matrix(); red.rows[0].status = 'RED';
  assert.equal(evaluateParityMatrix(red, source).status, 'fail');
  red.rows[0].ownerVisibleDisposition = 'Owner accepted at evidence/decision-20261005';
  assert.equal(evaluateParityMatrix(red, source).status, 'pass');
  red.rows.push({ id: 'B211-NEW', status: 'RED', evidence: 'external/observation' });
  assert.equal(evaluateParityMatrix(red, source).status, 'fail');
  for (const mutate of [m => m.rows.pop(), m => m.rows.push(m.rows[0]), m => m.sourceTree = 'c'.repeat(40), m => m.rows[0].evidence = '']) {
    const m = matrix(); mutate(m); assert.notEqual(evaluateParityMatrix(m, source).status, 'pass');
  }
});

test('MP-11 executable signal cleanup in test-only paths remains security-critical', () => {
  assert.deepEqual(classifySecurityPath('scripts/new-helper.test.mjs', 'process.kill(-pid, "SIGKILL")'), ['signals']);
  assert.deepEqual(classifySecurityPath('apps/kernel/src/tests/ordinary.rs', 'let value = 1;'), []);
  assert.ok(classifySecurityPath('apps/app-worker/worker.entitlements').includes('sandbox'));
});
