import assert from 'node:assert/strict'
import { readFile, mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'

const source = await readFile(new URL('./prepare-hetzner-image.sh', import.meta.url), 'utf8')
const functions = source.slice(source.indexOf('assert_unmounted_probe_root()'), source.indexOf('assert_path1_unit_has_no_dropins()'))
const linuxTest = process.platform === 'linux' ? test : test.skip
async function run(t, body) {
  const root = await mkdtemp(join(tmpdir(), 'chariox-probe-cleanup-'))
  t.after(() => rm(root, { recursive: true, force: true }))
  await mkdir(join(root, 'data'))
  await writeFile(join(root, 'outside'), 'preserve')
  const result = spawnSync('sh', ['-c', `set -eu\nfail() { echo "$*" >&2; exit 1; }\n${functions}\n${body}`, 'test', root], { encoding: 'utf8' })
  return { result, root }
}
linuxTest('fresh engine metadata is removed without following external symlinks', async t => {
  const { result, root } = await run(t, 'claim_empty_probe_root "$1/data"; mkdir "$1/data/volumes"; touch "$1/data/volumes/metadata.db"; ln -s "$1/outside" "$1/data/link"; clear_owned_probe_root "$1/data"; test -z "$(ls -A "$1/data")"')
  assert.equal(result.status, 0, result.stderr)
  assert.equal(await readFile(join(root, 'outside'), 'utf8'), 'preserve')
})
linuxTest('inherited data is refused before any probe cleanup', async t => {
  const { result, root } = await run(t, 'touch "$1/data/existing"; claim_empty_probe_root "$1/data"; clear_owned_probe_root "$1/data"')
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /pre-existing data/)
  await readFile(join(root, 'data/existing'))
})
linuxTest('a substituted directory is not cleaned', async t => {
  const { result, root } = await run(t, 'claim_empty_probe_root "$1/data"; mv "$1/data" "$1/original"; mkdir "$1/data"; touch "$1/data/preserve"; clear_owned_probe_root "$1/data"')
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /identity changed/)
  await readFile(join(root, 'data/preserve'))
})
linuxTest('mount discovery failure refuses cleanup', async t => {
  const { result } = await run(t, 'claim_empty_probe_root "$1/data"; findmnt() { return 1; }; clear_owned_probe_root "$1/data"')
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /could not inspect probe mounts/)
})
linuxTest('nested mounts refuse cleanup', async t => {
  const { result } = await run(t, 'claim_empty_probe_root "$1/data"; findmnt() { printf "%s\\n" "$probe_root/nested"; }; clear_owned_probe_root "$1/data"')
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /contains a mount/)
})
linuxTest('mount parser failure preserves probe data', async t => {
  const { result, root } = await run(t, 'claim_empty_probe_root "$1/data"; touch "$1/data/preserve"; awk() { return 2; }; clear_owned_probe_root "$1/data"')
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /could not parse probe mounts/)
  await readFile(join(root, 'data/preserve'))
})
test('production claims before start and cleans after verified shutdown', () => {
  const claim = source.indexOf('claim_empty_probe_root /var/lib/chariox-docker/data')
  const start = source.indexOf('systemctl start chariox-rootless-docker.service', claim)
  const stopped = source.indexOf('rootless Docker remained active while freezing the image', start)
  const clear = source.indexOf('clear_owned_probe_root /var/lib/chariox-docker/data', stopped)
  assert.ok(claim > 0 && start > claim && stopped > start && clear > stopped)
})
