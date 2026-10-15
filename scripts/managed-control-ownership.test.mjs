// MP-03/MP-07/MP-11: rejected and rolled-back activation preserves legacy access.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { chmod, chown, readFile, stat } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { makeHarness, put } from './lib/managed-kernel-upgrade-fixture.mjs'

// MP-03/MP-07: exercise real ownership under the same root contract as the updater.
if (process.platform === 'linux' && process.getuid() !== 0) {
  test('MP-03/MP-07 root-owned control fixtures', () => {
    const result = spawnSync('sudo', ['--', process.execPath, '--test', fileURLToPath(import.meta.url)], {
      encoding: 'utf8', maxBuffer: 16 * 1024 * 1024,
      env: { ...process.env, NODE_TEST_CONTEXT: undefined },
    })
    process.stdout.write(result.stdout ?? '')
    assert.equal(result.error, undefined)
    assert.equal(result.signal, null)
    assert.equal(result.status, 0, result.stderr)
  })
} else {
const transition = JSON.parse(await readFile(new URL('../apps/kernel/managed-upgrade-protocol-transitions.json', import.meta.url)))
async function harness(context) {
  const h = await makeHarness(context, { updaterPath: process.env.MP03_UPDATER, path1Release: true, currentProtocol: 370,
    targetProtocol: transition.protocol, targetTransitionPolicy: transition })
  const parent = dirname(h.receiptPath)
  await chown(parent, h.charioxIdentity.uid, h.charioxIdentity.gid)
  await chmod(parent, 0o700)
  await chown(h.receiptPath, h.charioxIdentity.uid, h.charioxIdentity.gid)
  await chmod(h.receiptPath, 0o600)
  for (const name of ['bootstrap-grant-binding.json', 'release-override.json']) {
    if (name === 'release-override.json') continue
    const path = join(parent, name)
    await put(path, '{"schemaVersion":1}\n', 0o600)
    await chown(path, h.charioxIdentity.uid, h.charioxIdentity.gid)
  }
  h.env = { CHARIOX_MANAGED_RELEASE_UPDATE_ID: 'managed_release_update_0123abcd-0000-4000-8000-0123456789ab', CHARIOX_MANAGED_PROVIDER_TOPOLOGY: 'path1',
    CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: h.trustedBuilderKey,
    CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY: h.nextTrustedBuilderKey }
  h.metadata = async () => Promise.all([parent, h.receiptPath, join(parent, 'bootstrap-grant-binding.json')].map(async path => {
    const s = await stat(path); return { uid: s.uid, gid: s.gid, mode: s.mode & 0o7777 }
  }))
  return h
}
for (const failure of ['reject-before-journal', 'rollback', 'interrupted']) {
  test(`MP-03/MP-07 ${failure} restores legacy control ownership`, async context => {
    const h = await harness(context)
    const before = await h.metadata()
    if (failure === 'reject-before-journal') {
      await put(join(h.state, 'home-drop-in'), 'reject\n')
    } else {
      await put(join(h.state, failure === 'rollback' ? 'fail-health-once' : 'crash-after-phase-stopped'), 'fail\n')
    }
    const result = h.run(h.env)
    assert.notEqual(result.status, 0)
    if (failure === 'interrupted') {
      const recovery = h.run({ ...h.env, CHARIOX_MANAGED_UPGRADE_RECOVER_ONLY: '1' })
      assert.equal(recovery.status, 0, recovery.stderr)
    }
    assert.deepEqual(await h.metadata(), before)
  })
}
test('MP-03 successful activation makes control state root-private', async context => {
  const h = await harness(context)
  const result = h.run(h.env)
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(await h.metadata(), [{uid:0,gid:0,mode:0o700}, {uid:0,gid:0,mode:0o600}, {uid:0,gid:0,mode:0o600}])
})

// A disposable worker's v1 unit declares StateDirectory=chariox, so its whole
// state root is service-owned and systemd re-owns it on every v1 start.
async function workerHarness(context, targetServicePolicy) {
  const h = await makeHarness(context, { updaterPath: process.env.MP03_UPDATER, path1Release: true, currentProtocol: 370,
    targetProtocol: transition.protocol, targetTransitionPolicy: transition, receiptKind: 'allocation_worker',
    currentServicePolicy: 1, targetServicePolicy })
  const parent = dirname(h.receiptPath), stateRoot = dirname(parent)
  for (const path of [stateRoot, parent]) {
    await chown(path, h.charioxIdentity.uid, h.charioxIdentity.gid)
    await chmod(path, 0o700)
  }
  await chown(h.receiptPath, h.charioxIdentity.uid, h.charioxIdentity.gid)
  await chmod(h.receiptPath, 0o600)
  h.env = { CHARIOX_MANAGED_PROVIDER_TOPOLOGY: 'path1', CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: h.trustedBuilderKey,
    CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY: h.nextTrustedBuilderKey }
  h.metadata = async () => Promise.all([stateRoot, parent, h.receiptPath].map(async path => {
    const s = await stat(path); return { uid: s.uid, gid: s.gid, mode: s.mode & 0o7777 }
  }))
  return h
}
test('MP-03 policy-2 worker activation makes the service-owned state root root-owned', async context => {
  const h = await workerHarness(context, 2)
  const result = h.run(h.env)
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(await h.metadata(), [{uid:0,gid:0,mode:0o755}, {uid:0,gid:0,mode:0o700}, {uid:0,gid:0,mode:0o600}])
})
for (const failure of ['rollback', 'interrupted']) {
  test(`MP-03/MP-07 policy-2 worker ${failure} restores the service-owned state root`, async context => {
    const h = await workerHarness(context, 2)
    const before = await h.metadata()
    if (failure === 'interrupted') {
      // Workers recover on the next updater run; that run then fails health,
      // so the final state is original only if recovery and rollback both restore it.
      await put(join(h.state, 'crash-after-supervisor-start'), 'fail\n')
      assert.notEqual(h.run(h.env).status, 0)
      assert.equal((await h.metadata())[0].uid, 0)
    }
    await put(join(h.state, 'fail-health-once'), 'fail\n')
    assert.notEqual(h.run(h.env).status, 0)
    assert.deepEqual(await h.metadata(), before)
  })
}
test('MP-03/MP-07 a StateDirectory target keeps service-principal worker access', async context => {
  const h = await workerHarness(context, 1)
  const before = await h.metadata()
  const result = h.run(h.env)
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(await h.metadata(), before)
})

}
