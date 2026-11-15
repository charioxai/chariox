import assert from 'node:assert/strict'
import { generateKeyPairSync, sign, createHash } from 'node:crypto'
import { mkdtemp, readFile, rm, mkdir, writeFile, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'

const sha = bytes => `sha256:${createHash('sha256').update(bytes).digest('hex')}`
const names = ['chariox-kernel', 'chariox-managed-bootstrap', 'chariox-managed-bootstrap.service', 'chariox-app-wasm-worker', 'chariox-build-attestation', 'chariox-build-attestation-signature', 'chariox-builder-public-key', 'chariox-enrollment-observer']
const paths = ['/usr/local/bin/chariox-kernel', '/usr/local/bin/chariox-managed-bootstrap', '/etc/systemd/system/chariox-managed-bootstrap.service', '/usr/local/bin/chariox-app-wasm-worker', '/usr/lib/chariox/build-attestation.json', '/usr/lib/chariox/build-attestation.sig', '/usr/lib/chariox/builder-public-key', '/usr/lib/chariox/enrollment-evidence.mjs']
const { privateKey, publicKey } = generateKeyPairSync('ed25519')
const key = publicKey.export({ type: 'spki', format: 'der' }).subarray(-32).toString('base64')
async function fixture(t, alter = () => {}) {
  const root = await mkdtemp(join(tmpdir(), 'boot-release-'))
  t.after(() => rm(root, { recursive: true, force: true }))
  await mkdir(join(root, 'etc/chariox/boot'), { recursive: true })
  await writeFile(join(root, 'etc/chariox/boot/release-public-key'), key)
  const files = new Map(names.map((name, i) => [`artifacts/${name}`, Buffer.from(i === 2 ? '[Service]\nExecStart=/usr/local/bin/chariox-managed-bootstrap\n' : `test-${name}`)]))
  const manifest = Buffer.from(JSON.stringify({ schemaVersion: 2, sourceCommit: 'a'.repeat(40), sourceTree: 'b'.repeat(40), artifacts: names.map((name, i) => ({ name, path: paths[i], sha256: sha(files.get(`artifacts/${name}`)) })) }))
  files.set('release-manifest.json', manifest)
  files.set('release-manifest.sig', Buffer.from(sign(null, manifest, privateKey).toString('base64')))
  alter(files)
  let activated = 0
  const options = { root, url: 'https://releases.example/immutable/', digest: sha(manifest), fetcher: async url => { const bytes = files.get(url.pathname.replace('/immutable/', '')); return new Response(bytes, { status: bytes ? 200 : 404 }) }, activate: async () => { activated++ } }
  return { root, files, options, activated: () => activated }
}

test('authenticated release installs then activates only the common bootstrap unit', async t => {
  const { installBootRelease } = await import('./boot-release.mjs')
  const f = await fixture(t)
  await installBootRelease(f.options)
  assert.equal(await readFile(join(f.root, 'usr/local/bin/chariox-kernel'), 'utf8'), 'test-chariox-kernel')
  assert.equal(f.activated(), 1)
  assert.equal((await stat(join(f.root, 'usr/lib/chariox/current'))).mode & 0o777, 0o755)
  await assert.rejects(installBootRelease(f.options), /already has a release/)
  assert.equal(f.activated(), 1)
})
test('the WASM worker installs read-only beside the kernel and a manifest without it is refused', async t => {
  const { installBootRelease } = await import('./boot-release.mjs')
  const f = await fixture(t)
  await installBootRelease(f.options)
  assert.equal((await stat(join(f.root, 'usr/lib/chariox/current/usr/local/bin/chariox-app-wasm-worker'))).mode & 0o777, 0o555)
  assert.equal((await stat(join(f.root, 'usr/lib/chariox/current/usr/local/bin/chariox-kernel'))).mode & 0o777, 0o755)
  const without = await fixture(t, files => {
    const manifest = JSON.parse(files.get('release-manifest.json'))
    manifest.artifacts = manifest.artifacts.filter(artifact => artifact.name !== 'chariox-app-wasm-worker')
    const bytes = Buffer.from(JSON.stringify(manifest))
    files.set('release-manifest.json', bytes)
    files.set('release-manifest.sig', Buffer.from(sign(null, bytes, privateKey).toString('base64')))
  })
  await assert.rejects(installBootRelease({ ...without.options, digest: sha(without.files.get('release-manifest.json')) }), /unsupported boot release manifest/)
  assert.equal(without.activated(), 0)
})
for (const target of ['release-manifest.json', 'release-manifest.sig', 'artifacts/chariox-kernel', 'artifacts/chariox-managed-bootstrap.service']) {
  test(`tampered ${target} cannot install or activate`, async t => {
    const { installBootRelease } = await import('./boot-release.mjs')
    const f = await fixture(t, files => files.set(target, Buffer.from('tampered')))
    await assert.rejects(installBootRelease(f.options))
    assert.equal(f.activated(), 0)
    await assert.rejects(readFile(join(f.root, 'usr/local/bin/chariox-kernel')))
  })
}
test('HTTP, credentials, fragments, mutable redirects and excessive bodies are refused', async t => {
  const { installBootRelease } = await import('./boot-release.mjs')
  for (const url of ['http://releases.example/immutable/', 'https://user:pass@releases.example/', 'https://releases.example/#latest']) {
    const f = await fixture(t)
    await assert.rejects(installBootRelease({ ...f.options, url }))
  }
  const f = await fixture(t)
  await assert.rejects(installBootRelease({ ...f.options, fetcher: async () => new Response(null, { status: 302, headers: { location: 'https://other.example/latest' } }) }))
  await assert.rejects(installBootRelease({ ...f.options, fetcher: async () => new Response(Buffer.alloc(65537)) }))
  assert.equal(f.activated(), 0)
})
