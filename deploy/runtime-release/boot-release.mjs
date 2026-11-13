#!/usr/bin/env node
// This verifier is installed in the base image, never downloaded with a release.
import { createHash, createPublicKey, verify } from 'node:crypto'
import { createWriteStream } from 'node:fs'
import { chmod, lstat, mkdir, mkdtemp, readFile, rename, rm, symlink, writeFile } from 'node:fs/promises'
import { dirname, join, relative, resolve } from 'node:path'
import { pipeline } from 'node:stream/promises'
import { Transform } from 'node:stream'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'
import { execFile } from 'node:child_process'

const run = promisify(execFile)
const spki = Buffer.from('302a300506032b6570032100', 'hex')
export const bootArtifacts = new Map([
  ['chariox-kernel', '/usr/local/bin/chariox-kernel'],
  ['chariox-managed-bootstrap', '/usr/local/bin/chariox-managed-bootstrap'],
  ['chariox-enrollment-observer', '/usr/lib/chariox/enrollment-evidence.mjs'],
  ['chariox-managed-bootstrap.service', '/etc/systemd/system/chariox-managed-bootstrap.service'],
  // The kernel opens its WASM App worker beside its own executable (WasmRuntime::beside_kernel).
  ['chariox-app-wasm-worker', '/usr/local/bin/chariox-app-wasm-worker'],
  ['chariox-build-attestation', '/usr/lib/chariox/build-attestation.json'],
  ['chariox-build-attestation-signature', '/usr/lib/chariox/build-attestation.sig'],
  ['chariox-builder-public-key', '/usr/lib/chariox/builder-public-key'],
])
const sha = bytes => `sha256:${createHash('sha256').update(bytes).digest('hex')}`
const fail = message => { throw new Error(message) }

function releaseUrl(value) {
  const url = new URL(value)
  if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash || !url.pathname.endsWith('/')) {
    fail('release URL must be an immutable HTTPS directory without credentials, query or fragment')
  }
  return url
}
async function response(fetcher, url) {
  const result = await fetcher(url, { redirect: 'error', signal: AbortSignal.timeout(120000) })
  if (result.status !== 200 || result.redirected || !result.body) fail('release download failed; retry the pinned release')
  return result
}
async function bounded(fetcher, url, limit) {
  const result = await response(fetcher, url)
  const chunks = []
  let size = 0
  for await (const chunk of result.body) {
    size += chunk.length
    if (size > limit) fail('release metadata exceeds its limit')
    chunks.push(chunk)
  }
  return Buffer.concat(chunks)
}
function authenticate(bytes, signature, trusted, digest) {
  if (!/^sha256:[0-9a-f]{64}$/.test(digest) || sha(bytes) !== digest) fail('release manifest digest mismatch')
  const key = trusted.toString().trim()
  const sig = signature.toString().trim()
  if (!/^[A-Za-z0-9+/]{43}=$/.test(key) || !/^[A-Za-z0-9+/]{86}==$/.test(sig)) fail('invalid release trust material')
  const publicKey = createPublicKey({ key: Buffer.concat([spki, Buffer.from(key, 'base64')]), format: 'der', type: 'spki' })
  if (!verify(null, bytes, publicKey, Buffer.from(sig, 'base64'))) fail('release manifest signature mismatch')
  const manifest = JSON.parse(bytes)
  if (manifest.schemaVersion !== 2 || !/^[0-9a-f]{40}$/.test(manifest.sourceCommit) || !/^[0-9a-f]{40}$/.test(manifest.sourceTree) ||
      Object.keys(manifest).sort().join(',') !== 'artifacts,schemaVersion,sourceCommit,sourceTree' ||
      !Array.isArray(manifest.artifacts) || manifest.artifacts.length !== bootArtifacts.size) fail('unsupported boot release manifest')
  const seen = new Set()
  for (const artifact of manifest.artifacts) {
    if (Object.keys(artifact).sort().join(',') !== 'name,path,sha256' || seen.has(artifact.name) ||
        bootArtifacts.get(artifact.name) !== artifact.path || !/^sha256:[0-9a-f]{64}$/.test(artifact.sha256)) fail('invalid boot release artifact')
    seen.add(artifact.name)
  }
  for (const name of bootArtifacts.keys()) if (!seen.has(name)) fail('required boot artifact missing')
  return manifest
}
async function realDirectories(path) {
  const parent = dirname(path)
  if (parent !== path) await realDirectories(parent)
  try { await mkdir(path, { mode: 0o755 }) } catch (error) { if (error.code !== 'EEXIST') throw error }
  const stat = await lstat(path)
  if (!stat.isDirectory() || stat.isSymbolicLink()) fail('boot destination has an unsafe ancestor')
}
async function downloadArtifact(fetcher, url, path, digest) {
  const result = await response(fetcher, url)
  const hash = createHash('sha256')
  let bytes = 0
  const check = new Transform({ transform(chunk, encoding, callback) {
    bytes += chunk.length
    if (bytes > 1024 ** 3) return callback(new Error('release artifact exceeds its limit'))
    hash.update(chunk)
    callback(null, chunk)
  } })
  await realDirectories(dirname(path))
  await pipeline(result.body, check, createWriteStream(path, { flags: 'wx', mode: 0o600 }))
  if (`sha256:${hash.digest('hex')}` !== digest) fail('release artifact digest mismatch')
}

export async function installBootRelease({ url, digest, root = '/', fetcher = fetch, activate }) {
  const base = releaseUrl(url)
  root = resolve(root)
  const control = join(root, 'etc/chariox/boot')
  await realDirectories(control)
  const keyPath = join(control, 'release-public-key')
  const stat = await lstat(keyPath)
  if (stat.uid !== 0 || !stat.isFile() || stat.isSymbolicLink() || stat.size > 1024 || stat.mode & 0o022) fail('image-pinned public trust root is unsafe')
  const trusted = await readFile(keyPath)
  // One image-pinned lock serializes installs and prevents overlapping activation.
  const lock = join(control, 'install.lock')
  await mkdir(lock).catch(() => fail('boot install already active; inspect the previous install before retrying'))
  let staging
  try {
    for (const installed of ['usr/lib/chariox/current', 'usr/local/bin/chariox-kernel', 'usr/local/bin/chariox-managed-bootstrap']) {
      const present = await lstat(join(root, installed)).then(() => true, error => { if (error.code === 'ENOENT') return false; throw error })
      if (present) fail('allocation already has a release; reboot its pinned bootstrap unit instead of reinstalling')
    }
    const bytes = await bounded(fetcher, new URL('release-manifest.json', base), 65536)
    const signature = await bounded(fetcher, new URL('release-manifest.sig', base), 1024)
    const manifest = authenticate(bytes, signature, trusted, digest)
    const library = join(root, 'usr/lib/chariox')
    await realDirectories(join(library, 'releases'))
    staging = await mkdtemp(join(library, 'releases/.boot-'))
    for (const artifact of manifest.artifacts) {
      const path = join(staging, artifact.path)
      await downloadArtifact(fetcher, new URL(`artifacts/${artifact.name}`, base), path, artifact.sha256)
      // The worker is read-only so WasmRuntime::open trusts it for a root or a service-user kernel.
      await chmod(path, artifact.name === 'chariox-app-wasm-worker' ? 0o555 : artifact.path.startsWith('/usr/local/bin/') ? 0o755 : 0o644)
    }
    const metadata = join(staging, 'usr/lib/chariox')
    await realDirectories(metadata)
    await writeFile(join(metadata, 'release-manifest.json'), bytes, { mode: 0o644 })
    await writeFile(join(metadata, 'release-manifest.sig'), signature, { mode: 0o644 })
    await writeFile(join(metadata, 'release-public-key'), trusted, { mode: 0o644 })
    await chmod(staging, 0o755)
    const destination = join(library, 'releases', digest.slice(7))
    // A fresh allocation installs once. Reboots use the installed pinned release.
    await rename(staging, destination)
    staging = undefined
    const facades = [...manifest.artifacts.map(a => a.path), ...['release-manifest.json', 'release-manifest.sig', 'release-public-key'].map(n => `/usr/lib/chariox/${n}`)]
    for (const facade of facades) {
      const path = join(root, facade)
      await realDirectories(dirname(path))
      const target = relative(dirname(path), join(library, 'current', facade))
      await symlink(target, `${path}.boot-new`)
      await rename(`${path}.boot-new`, path)
    }
    await symlink(`releases/${digest.slice(7)}`, join(library, 'current.boot-new'))
    await rename(join(library, 'current.boot-new'), join(library, 'current'))
    if (activate) await activate()
    else {
      if (root !== '/') fail('service activation requires the real root')
      await run('/usr/bin/systemctl', ['daemon-reload'])
      await run('/usr/bin/systemctl', ['enable', '--now', 'chariox-managed-bootstrap.service'])
    }
    return { digest, sourceCommit: manifest.sourceCommit }
  } finally {
    if (staging) await rm(staging, { recursive: true, force: true })
    await rm(lock, { recursive: true })
  }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 4 || process.getuid() !== 0) {
    console.error('usage (root): boot-release.mjs <immutable-https-release-directory/> <sha256:manifest-digest>')
    process.exitCode = 1
  } else {
    installBootRelease({ url: process.argv[2], digest: process.argv[3] }).then(result => console.log(JSON.stringify(result))).catch(() => {
      // Network errors can contain request data; keep the public failure fixed.
      console.error('authenticated boot install failed; do not enroll; inspect operator diagnostics and delete the failed allocation')
      process.exitCode = 1
    })
  }
}
