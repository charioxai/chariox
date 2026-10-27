// MP-08 / MP-09 / MP-10 / MP-11: cleanup owns only a successfully created home.
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { Fixture } from './fixture.mjs'

test('a refused existing home is never owned by fixture cleanup', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'room-exec-fixture-'))
  const home = path.join(root, 'existing-state'), evidence = path.join(root, 'evidence')
  await mkdir(home); await mkdir(evidence)
  const marker = path.join(home, 'keep')
  await writeFile(marker, 'existing state')
  const fixture = new Fixture({ home, evidence, kernelBinary: '/unused', relayBinary: '/unused' })
  try {
    await assert.rejects(fixture.start(), { code: 'EEXIST' })
    await fixture.cleanup()
    assert.equal(await readFile(marker, 'utf8'), 'existing state')
  } finally { await rm(root, { recursive: true, force: true }) }
})
