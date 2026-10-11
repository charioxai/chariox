import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, chmod, rm } from 'node:fs/promises'
import { readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'

// MP-03/MP-07: preparation must accept exactly the empty root-private control
// roots that install-image.sh creates, and nothing else under the state root.
const source = await readFile(new URL('./prepare-hetzner-image.sh', import.meta.url), 'utf8')
const start = source.indexOf('require_pristine_state_root() {')
assert.ok(start >= 0, 'preparation must define the pristine state-root check')
const helper = source.slice(start, source.indexOf('\n}\n', start) + 3)
assert.match(source, /^require_pristine_state_root \/var\/lib\/chariox$/m, 'preparation must apply the check to the image state root')

async function check(t, layout) {
  const root = await mkdtemp(join(tmpdir(), 'chariox-image-state-root-'))
  t.after(() => rm(root, { recursive: true, force: true }))
  const stateRoot = join(root, 'var/lib/chariox')
  await mkdir(stateRoot, { recursive: true, mode: 0o755 })
  await layout(stateRoot)
  // Unprivileged runs map the test user to root; ownership itself is checked by
  // the root-only case below.
  const uid = process.getuid()
  const result = spawnSync('sh', ['-c', `set -eu
fail() { echo "prepare-hetzner-image.sh: $*" >&2; exit 1; }
stat() { if [ "$1" = -c ] && [ "$2" = '%u:%a' ]; then o=$(command stat -c '%u:%a' "$3"); [ "\${o%%:*}" = ${uid} ] && o=0:\${o#*:}; printf '%s\\n' "$o"; else command stat "$@"; fi; }
${helper}
require_pristine_state_root "$1"`, 'test', stateRoot], { encoding: 'utf8', timeout: 10_000 })
  return result
}

const installerLayout = async stateRoot => {
  for (const name of ['managed', 'disposable-worker']) {
    await mkdir(join(stateRoot, name))
    await chmod(join(stateRoot, name), 0o700)
  }
}

test('installer-created empty control roots pass image preparation', async t => {
  const result = await check(t, installerLayout)
  assert.equal(result.status, 0, result.stderr)
})

test('an empty state root still passes', async t => {
  const result = await check(t, async () => {})
  assert.equal(result.status, 0, result.stderr)
})

test('a receipt inside a control root is runtime state', async t => {
  const result = await check(t, async stateRoot => {
    await installerLayout(stateRoot)
    await writeFile(join(stateRoot, 'managed/bootstrap-receipt.json'), '{}')
  })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /control state root is not empty and root-private/)
})

test('a non-private control root is rejected', async t => {
  const result = await check(t, async stateRoot => {
    await installerLayout(stateRoot)
    await chmod(join(stateRoot, 'managed'), 0o755)
  })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /control state root is not empty and root-private/)
})

test('any other state-root entry is rejected', async t => {
  const result = await check(t, async stateRoot => {
    await installerLayout(stateRoot)
    await writeFile(join(stateRoot, 'managed-bootstrap.json'), '{}')
  })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed runtime state entered the image/)
})

const rootTest = process.getuid?.() === 0 ? test : test.skip
rootTest('a control root owned by the service account is rejected', async t => {
  const result = await check(t, async stateRoot => {
    await installerLayout(stateRoot)
    spawnSync('chown', ['65534:65534', join(stateRoot, 'disposable-worker')])
  })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /control state root is not empty and root-private/)
})
