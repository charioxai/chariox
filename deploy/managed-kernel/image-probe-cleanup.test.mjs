import assert from 'node:assert/strict'
import { readFile, mkdtemp, mkdir, writeFile, rm, symlink, readdir, readlink } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'

const source = await readFile(new URL('./prepare-hetzner-image.sh', import.meta.url), 'utf8')
const functions = source.slice(source.indexOf('assert_unmounted_probe_root()'), source.indexOf('assert_path1_unit_has_no_dropins()'))
const linuxTest = process.platform === 'linux' ? test : test.skip
async function runExit(t, body, scenario = '', topology = 'path1') {
  const root = await mkdtemp(join(tmpdir(), 'chariox-probe-exit-'))
  t.after(() => rm(root, { recursive: true, force: true }))
  await mkdir(join(root, 'data'))
  for (const unit of ['chariox-rootless-docker', 'chariox-slice-disk-quota-allocator']) {
    const dir = join(root, 'etc', `${unit}.service.d`)
    await mkdir(dir, { recursive: true })
    await symlink(`../../../../usr/lib/chariox/current/etc/systemd/system/${unit}.service.d/50-chariox-data-volume.conf`, join(dir, '50-chariox-data-volume.conf'))
  }
  const shell = source.slice(source.indexOf('fail()'), source.indexOf('\nif [ "$(id -u)"'))
    .replaceAll('/etc/systemd/system/', `${root}/etc/`)
    // Targets are signed relative strings, not fixture paths.
    .replaceAll(`../../../../usr/lib/chariox/current${root}/etc/`, '../../../../usr/lib/chariox/current/etc/systemd/system/')
    .replaceAll('/var/lib/chariox-docker/data', `${root}/data`)
  const result = spawnSync('sh', ['-c', `set -eu
path1_data_volume_dropins_bypassed=0
managed_provider_topology=${topology}
${shell}
systemctl() {
  printf '%s\\n' "$*" >> '${root}/services'
  if [ '${topology}' = shared_host ]; then
    case "$*" in *chariox-data-volume-admission.service*) return 5 ;; esac
  fi
  case "$1" in
    stop) [ '${scenario}' != stop-failure ] ;;
    show) [ '${scenario}' != state-error ] || return 1; if [ '${scenario}' = active ]; then echo active; else echo inactive; fi ;;
    is-active) [ '${scenario}' = active ] ;;
    *) return 0 ;;
  esac
}
findmnt() { printf '/\\n'; }
stat() { '${process.execPath}' -e 'const s=require("node:fs").statSync(process.argv[1]); console.log(s.dev+":"+s.ino)' "$3"; }
bypass_path1_data_volume_dropins
${body}`, 'test', root], { encoding: 'utf8', timeout: 10_000 })
  return { root, result }
}

test('EXIT after a claimed probe failure clears only its data and preserves the original failure', async t => {
  const { root, result } = await runExit(t, 'claim_empty_probe_root "$1/data"; touch "$1/data/probe-metadata"; exit 7')
  assert.equal(result.status, 7, result.stderr)
  assert.deepEqual(await readdir(join(root, 'data')), [])
  assert.equal(await readlink(join(root, 'etc/chariox-rootless-docker.service.d/50-chariox-data-volume.conf')),
    '../../../../usr/lib/chariox/current/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf')
  const services = await readFile(join(root, 'services'), 'utf8')
  for (const unit of ['chariox-rootless-docker', 'chariox-slice-disk-quota-allocator', 'chariox-data-volume-admission']) {
    assert.ok(services.includes(`show --property=ActiveState --value ${unit}.service`))
  }
})

test('shared-host EXIT clears only claimed probe data without requiring Path-1 admission', async t => {
  const { root, result } = await runExit(t, 'claim_empty_probe_root "$1/data"; touch "$1/data/probe-metadata"; exit 7', '', 'shared_host')
  assert.equal(result.status, 7, result.stderr)
  assert.deepEqual(await readdir(join(root, 'data')), [])
  const services = await readFile(join(root, 'services'), 'utf8')
  for (const unit of ['chariox-rootless-docker', 'chariox-slice-disk-quota-allocator']) {
    assert.ok(services.includes(`stop ${unit}.service`))
    assert.ok(services.includes(`show --property=ActiveState --value ${unit}.service`))
  }
  assert.doesNotMatch(services, /chariox-data-volume-admission|daemon-reload/)
})

test('shared-host inherited data is never claimed or cleared and does not stop services', async t => {
  const { root, result } = await runExit(t, 'touch "$1/data/inherited"; claim_empty_probe_root "$1/data"', '', 'shared_host')
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /pre-existing data/)
  assert.deepEqual(await readdir(join(root, 'data')), ['inherited'])
  assert.ok(!(await readdir(root)).includes('services'))
})

for (const [scenario, mutation, diagnostic] of [
  ['active', '', /storage services could not be stopped/],
  ['stop-failure', '', /storage services could not be stopped/],
  ['state-error', '', /storage services could not be stopped/],
  ['', 'mv "$1/data" "$1/original"; mkdir "$1/data"; touch "$1/data/unowned"', /identity changed/],
  ['', 'findmnt() { printf "%s\\n" "$probe_root/nested"; }', /contains a mount/],
  ['', 'findmnt() { return 1; }', /could not inspect probe mounts/],
]) {
  test(`shared-host cleanup preserves data on ${scenario || diagnostic.source}`, async t => {
    const { root, result } = await runExit(t,
      `claim_empty_probe_root "$1/data"; touch "$1/data/probe-metadata"; ${mutation || ':'}; exit 7`, scenario, 'shared_host')
    assert.equal(result.status, 7, result.stderr)
    assert.match(result.stderr, diagnostic)
    assert.ok((await readdir(join(root, 'data'))).length > 0)
  })
}

test('EXIT before claim preserves inherited data', async t => {
  const { root, result } = await runExit(t, 'touch "$1/data/inherited"; exit 7')
  assert.equal(result.status, 7, result.stderr)
  assert.deepEqual(await readdir(join(root, 'data')), ['inherited'])
})

test('failed empty-root claim never authorizes EXIT cleanup', async t => {
  const { root, result } = await runExit(t, 'touch "$1/data/inherited"; claim_empty_probe_root "$1/data"')
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /pre-existing data/)
  assert.deepEqual(await readdir(join(root, 'data')), ['inherited'])
})

for (const [name, mutation, diagnostic] of [
  ['changed inode', 'mv "$1/data" "$1/original"; mkdir "$1/data"; touch "$1/data/unowned"', /identity changed/],
  ['nested mount', 'findmnt() { printf "%s\\n" "$probe_root/nested"; }', /contains a mount/],
  ['mount census failure', 'findmnt() { return 1; }', /could not inspect probe mounts/],
]) {
  test(`EXIT refuses ${name}, restores drop-ins and preserves the original failure`, async t => {
    const { root, result } = await runExit(t, `claim_empty_probe_root "$1/data"; touch "$1/data/probe-metadata"; ${mutation}; exit 7`)
    assert.equal(result.status, 7, result.stderr)
    assert.match(result.stderr, diagnostic)
    assert.ok((await readdir(join(root, 'data'))).length > 0)
    await readlink(join(root, 'etc/chariox-rootless-docker.service.d/50-chariox-data-volume.conf'))
  })
}

for (const scenario of ['active', 'state-error', 'stop-failure']) for (const exit of [0, 7]) {
  test(`EXIT preserves probe data when ${scenario}, reporting failure from exit ${exit}`, async t => {
    const { root, result } = await runExit(t, `claim_empty_probe_root "$1/data"; touch "$1/data/probe-metadata"; exit ${exit}`, scenario)
    assert.equal(result.status, exit || 1, result.stderr)
    assert.match(result.stderr, /storage services could not be stopped/)
    assert.deepEqual(await readdir(join(root, 'data')), ['probe-metadata'])
  })
}

test('EXIT mount refusal turns an otherwise successful exit into failure', async t => {
  const { root, result } = await runExit(t, 'claim_empty_probe_root "$1/data"; touch "$1/data/probe-metadata"; findmnt() { return 1; }; exit 0')
  assert.equal(result.status, 1, result.stderr)
  assert.deepEqual(await readdir(join(root, 'data')), ['probe-metadata'])
})
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

test('a frozen Path-1 image keeps a root-owned mountpoint; shared host keeps a rootless-owned data root', () => {
  const clear = source.indexOf('clear_owned_probe_root /var/lib/chariox-docker/data')
  const gate = source.indexOf('if [ "$managed_provider_topology" = path1 ]; then', clear)
  const keep = source.indexOf('install -d -o root -g root -m 0700 /var/lib/chariox-docker/data', gate)
  const otherwise = source.indexOf('else', keep)
  const remove = source.indexOf('install -d -o chariox-docker -g chariox-docker -m 0700 /var/lib/chariox-docker/data', otherwise)
  assert.ok(clear > 0 && gate > clear && keep > gate && otherwise > keep && remove > otherwise)
  assert.ok(source.indexOf('\nfi\n', keep) > remove)
})
