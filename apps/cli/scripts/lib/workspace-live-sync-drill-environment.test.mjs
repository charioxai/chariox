import assert from 'node:assert/strict'
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'

import {
  prepareWorkspaceLiveSyncDaemonEnvironment,
  removeWorkspaceLiveSyncProviderProfile,
} from './workspace-live-sync-drill-environment.mjs'

test('isolates Chariox state and copies only provider credentials', async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'workspace-live-sync-env-'))
  const sourceHome = path.join(root, 'source-home')
  const runRoot = path.join(root, 'run')
  await mkdir(path.join(sourceHome, '.codex', 'sessions'), { recursive: true })
  await mkdir(path.join(sourceHome, '.local', 'share', 'opencode'), { recursive: true })
  await writeFile(path.join(sourceHome, '.codex', 'auth.json'), '{"token":"codex-test"}\n')
  await writeFile(path.join(sourceHome, '.codex', 'sessions', 'old.jsonl'), 'do not copy\n')
  await writeFile(
    path.join(sourceHome, '.local', 'share', 'opencode', 'auth.json'),
    '{"token":"opencode-test"}\n',
  )
  t.after(async () => await rm(root, { recursive: true, force: true }))

  const profile = await prepareWorkspaceLiveSyncDaemonEnvironment({
    rootDir: runRoot,
    daemonId: 'worker-1',
    providers: ['codex', 'opencode'],
    sourceEnv: {},
    sourceHome,
  })

  assert.notEqual(profile.env.HOME, sourceHome)
  assert.match(profile.env.XDG_STATE_HOME, /worker-1-xdg-state$/)
  assert.equal(
    await readFile(path.join(profile.env.CODEX_HOME, 'auth.json'), 'utf8'),
    '{"token":"codex-test"}\n',
  )
  assert.equal(
    await readFile(path.join(profile.env.OPENCODE_DATA_HOME, 'auth.json'), 'utf8'),
    '{"token":"opencode-test"}\n',
  )
  assert.equal((await stat(path.join(profile.env.CODEX_HOME, 'auth.json'))).mode & 0o777, 0o600)
  await assert.rejects(
    readFile(path.join(profile.env.CODEX_HOME, 'sessions', 'old.jsonl')),
    /ENOENT/,
  )

  await removeWorkspaceLiveSyncProviderProfile(profile)
  await assert.rejects(stat(profile.credentialRoot), /ENOENT/)
  assert.equal((await stat(profile.env.HOME)).isDirectory(), true)
})

test('honors explicit provider data roots without requiring credentials', async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'workspace-live-sync-env-explicit-'))
  t.after(async () => await rm(root, { recursive: true, force: true }))

  const profile = await prepareWorkspaceLiveSyncDaemonEnvironment({
    rootDir: path.join(root, 'run'),
    daemonId: 'worker-2',
    providers: ['codex', 'opencode'],
    sourceEnv: {
      CODEX_HOME: path.join(root, 'missing-codex'),
      OPENCODE_DATA_HOME: path.join(root, 'missing-opencode'),
    },
    sourceHome: path.join(root, 'home'),
  })

  await assert.rejects(readFile(path.join(profile.env.CODEX_HOME, 'auth.json')), /ENOENT/)
  await assert.rejects(readFile(path.join(profile.env.OPENCODE_DATA_HOME, 'auth.json')), /ENOENT/)
  await removeWorkspaceLiveSyncProviderProfile(profile)
})
