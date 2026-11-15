#!/usr/bin/env node
import { createHash, createPrivateKey, createPublicKey, sign } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { execFileSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { bootArtifacts } from '../deploy/runtime-release/boot-release.mjs'

// Run on the release signer. Lane drills use an ephemeral TEST key only.
const options = Object.fromEntries(process.argv.slice(2).reduce((pairs, arg, i, args) => i % 2 ? pairs : [...pairs, [arg, args[i + 1]]], []))
for (const name of ['kernel', 'bootstrap', 'enrollment-observer', 'bootstrap-unit', 'worker', 'relay', 'signing-key', 'source', 'output']) {
  if (!options[`--${name}`]) throw new Error(`missing --${name}`)
}
const output = resolve(options['--output'])
await mkdir(output, { recursive: false, mode: 0o755 })
await mkdir(join(output, 'artifacts'))
const key = createPrivateKey(await readFile(options['--signing-key']))
if (key.asymmetricKeyType !== 'ed25519') throw new Error('Ed25519 release signing key required')
const publicKey = createPublicKey(key).export({ format: 'der', type: 'spki' }).subarray(-32).toString('base64')
const source = resolve(options['--source'])
const git = args => execFileSync('git', ['-C', source, ...args], { encoding: 'utf8' }).trim()
if (git(['status', '--porcelain']).length) throw new Error('release source worktree must be clean')
const sourceCommit = git(['rev-parse', 'HEAD'])
const sourceTree = git(['rev-parse', 'HEAD^{tree}'])
const sha = bytes => `sha256:${createHash('sha256').update(bytes).digest('hex')}`
const kernel = await readFile(options['--kernel'])
const bootstrap = await readFile(options['--bootstrap'])
const worker = await readFile(options['--worker'])
const relay = await readFile(options['--relay'])
const attestation = Buffer.from(JSON.stringify({ schemaVersion: 1, sourceCommit, sourceTree, target: 'x86_64-unknown-linux-gnu', artifacts: [
  { name: 'chariox-kernel', sha256: sha(kernel) }, { name: 'chariox-managed-bootstrap', sha256: sha(bootstrap) }, { name: 'chariox-relay', sha256: sha(relay) },
] }) + '\n')
const directory = dirname(fileURLToPath(import.meta.url))
const contents = new Map([
  ['chariox-kernel', kernel], ['chariox-managed-bootstrap', bootstrap], ['chariox-app-wasm-worker', worker],
  ['chariox-enrollment-observer', await readFile(options['--enrollment-observer'])],
  ['chariox-managed-bootstrap.service', await readFile(options['--bootstrap-unit'])],
  ['chariox-build-attestation', attestation],
  ['chariox-build-attestation-signature', Buffer.from(sign(null, attestation, key).toString('base64') + '\n')],
  ['chariox-builder-public-key', Buffer.from(publicKey + '\n')],
])
const artifacts = []
for (const [name, path] of bootArtifacts) {
  const bytes = contents.get(name)
  await writeFile(join(output, 'artifacts', name), bytes, { mode: 0o644 })
  artifacts.push({ name, path, sha256: sha(bytes) })
}
const manifest = Buffer.from(JSON.stringify({ schemaVersion: 2, sourceCommit, sourceTree, artifacts }) + '\n')
await writeFile(join(output, 'release-manifest.json'), manifest)
await writeFile(join(output, 'release-manifest.sig'), sign(null, manifest, key).toString('base64') + '\n')
console.log(JSON.stringify({ digest: sha(manifest), sourceCommit, sourceTree, output }))
