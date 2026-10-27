import assert from 'node:assert/strict'
import { mkdtemp, readdir, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'

import {
  CHROMIUM_SANDBOX_PREFLIGHT_REASON,
  assertChromiumSandboxPrerequisites,
  chromiumSandboxProbeArguments,
  preflightChromiumSandbox,
} from './chromium-sandbox-preflight.mjs'

const privateDirectory = { directory: true, symbolicLink: false, uid: 1001, mode: 0o700, writable: true }

test('Chromium sandbox prerequisites require the non-root owner and private writable directories', () => {
  const valid = {
    platform: 'linux',
    uid: 1001,
    profile: privateDirectory,
    runtime: privateDirectory,
    setuidSandbox: false,
    userNamespaces: true,
  }
  assert.doesNotThrow(() => assertChromiumSandboxPrerequisites(valid))
  for (const input of [
    { ...valid, uid: 0 },
    { ...valid, profile: { ...privateDirectory, uid: 1002 } },
    { ...valid, runtime: { ...privateDirectory, writable: false } },
    { ...valid, runtime: { ...privateDirectory, symbolicLink: true } },
    { ...valid, profile: { ...privateDirectory, mode: 0o722 } },
    { ...valid, setuidSandbox: false, userNamespaces: false },
  ]) {
    assert.throws(
      () => assertChromiumSandboxPrerequisites(input),
      { message: new RegExp(`^${CHROMIUM_SANDBOX_PREFLIGHT_REASON}:`) },
    )
  }
  assert.doesNotThrow(() => assertChromiumSandboxPrerequisites({
    ...valid,
    setuidSandbox: true,
    userNamespaces: false,
  }))
})

test('sandbox initialization probe uses an isolated profile and no bypass arguments', () => {
  const args = chromiumSandboxProbeArguments('/private/runtime/probe-profile')
  assert.ok(args.includes('--headless=new'))
  assert.ok(args.includes('--dump-dom'))
  assert.ok(args.includes('--user-data-dir=/private/runtime/probe-profile'))
  assert.equal(args.some((arg) => arg === '--no-sandbox'), false)
  assert.equal(
    args.some((arg) => arg.startsWith('--unsafely-treat-insecure-origin-as-secure')),
    false,
  )
})

test('sandbox initialization failure is fixed, fail-closed, and cleans its probe profile', async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'chariox-chromium-preflight-'))
  t.after(() => rm(root, { recursive: true, force: true }))
  const profileDirectory = path.join(root, 'profile')
  const runtimeDirectory = path.join(root, 'runtime')
  let observed = null
  await assert.rejects(
    preflightChromiumSandbox({
      executable: 'fixture-chromium',
      profileDirectory,
      runtimeDirectory,
      platform: 'darwin',
      uid: 1001,
      inspect: async () => ({
        platform: 'darwin',
        uid: 1001,
        profile: privateDirectory,
        runtime: privateDirectory,
        setuidSandbox: false,
        userNamespaces: false,
      }),
      probe: async (executable, args, env) => {
        observed = { executable, args, runtime: env.XDG_RUNTIME_DIR }
        return { code: 1, timedOut: false }
      },
    }),
    { message: new RegExp(`^${CHROMIUM_SANDBOX_PREFLIGHT_REASON}: sandbox-initialization-failed;`) },
  )
  assert.equal(observed.executable, 'fixture-chromium')
  assert.equal(observed.runtime, runtimeDirectory)
  assert.equal(observed.args.some((arg) => arg === '--no-sandbox'), false)
  assert.deepEqual(await readdir(runtimeDirectory), [])
})
