import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const root = resolve(fileURLToPath(import.meta.url), '../..')
const read = path => readFileSync(resolve(root, path), 'utf8')

test('the runtime does not run the retired rental release updater', () => {
  for (const path of ['apps/kernel/src/runtime/managed_release_update.rs', 'apps/kernel/src/runtime/managed_release_update_evidence.rs']) {
    assert.equal(existsSync(resolve(root, path)), false, `${path} is Cloud policy`)
  }
  assert.doesNotMatch(read('apps/kernel/src/runtime_transport.rs'), /spawn_managed_release_update/)
  const paths = JSON.parse(read('scripts/control-plane-paths.allow.json'))
  assert.deepEqual(paths.filter(row => /\/release-update\/(poll|artifact)$/.test(row.path)), [])
})

test('enrollment identity observation has its own shared request builder', async () => {
  const { observeKernelRuntimeIdentityRequest } = await import('../packages/kernel-client/dist/ipc-runtime-identity-requests.js')
  assert.deepEqual(observeKernelRuntimeIdentityRequest({ environmentId: 'enrollment-1', expectedGeneration: 2 }), {
    ObserveKernelRuntimeIdentity: { environmentId: 'enrollment-1', expectedGeneration: 2 },
  })
  const paths = JSON.parse(read('scripts/control-plane-paths.allow.json'))
  assert.equal(paths.some(row => row.path === '/v1/managed-kernels/reimage/runtime-identity'), false)
  assert.equal(paths.some(row => row.path === '/v1/kernels/runtime-identity' && row.seam === 'kernel runtime identity observation'), true)
})

test('the TUI has no direct rental client or rental configuration controls', () => {
  assert.equal(existsSync(resolve(root, 'apps/cli/src/managed-environment-api.ts')), false)
  const paths = JSON.parse(read('scripts/control-plane-paths.allow.json'))
  assert.deepEqual(paths.filter(row => row.path.startsWith('/managed-environments')), [])
  const composition = read('apps/cli/src/cli-waiting-room-composition.ts')
  assert.match(composition, /acquireMachineAllocation\(deps.client/)
  assert.doesNotMatch(composition, /createManagedEnvironment|managedEnvironmentJson/)
})

test('deployment installation and upgrade tooling is supplied outside OSS', () => {
  for (const name of ['install-image.sh', 'upgrade-image.sh', 'chariox-managed-bootstrap.service', 'chariox-path1-managed-bootstrap.service']) {
    assert.equal(existsSync(resolve(root, 'deploy/managed-kernel', name)), false, `${name} belongs to the deployment authority`)
  }
})
