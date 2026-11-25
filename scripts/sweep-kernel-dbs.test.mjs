import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./sweep-kernel-dbs.mjs', import.meta.url))
const sweep = (...args) => spawnSync(process.execPath, [script, ...args], { encoding: 'utf8' })

test('reports database copies, even inside a copy of the state directory', t => {
  const root = mkdtempSync(join(tmpdir(), 'sweep-kernel-dbs-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const home = join(root, 'live')
  const paths = ['copied-home/state/kernel.db', 'evidence/kernel.db', 'live/state/kernel.db']
  for (const path of paths) {
    const db = join(root, path)
    mkdirSync(dirname(db), { recursive: true })
    writeFileSync(db, 'fixture')
  }
  symlinkSync(join(home, 'state', 'kernel.db'), join(root, 'kernel.db'))

  const result = sweep('--kernel-db', join(home, 'state', 'kernel.db'), root)
  assert.equal(result.status, 1, result.stderr)
  assert.deepEqual(result.stdout.trim().split('\n'), [
    ...paths.slice(0, 2), 'kernel.db',
  ].map(path => join(root, path)))

  for (const path of [...paths.slice(0, 2), 'kernel.db']) rmSync(join(root, path))
  const clean = sweep('--kernel-db', join(home, 'state', 'kernel.db'), root)
  assert.equal(clean.status, 0, clean.stderr)
  assert.equal(clean.stdout, '')
})

test('a missing scan root is an error, even when another root exists', t => {
  const root = mkdtempSync(join(tmpdir(), 'sweep-kernel-dbs-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const result = sweep(root, join(root, 'missing-kernel-db-scan-root'))
  assert.equal(result.status, 2, result.stderr)
  assert.match(result.stderr, /missing-kernel-db-scan-root/)
})
