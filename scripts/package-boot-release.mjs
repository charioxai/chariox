#!/usr/bin/env node
import { createHash, createPrivateKey, createPublicKey, sign } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { execFileSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { bootArtifacts } from '../deploy/managed-kernel/boot-release.mjs'
import { manifestBytes, signingRequestBytes } from './release-signing-documents.mjs'

// With --signing-key this signs in place (lane drills use an ephemeral TEST key only). With --signing-request it
// stages the release unsigned and writes the small signing request for sign-digests-on-mac.mjs.
const options = Object.fromEntries(process.argv.slice(2).reduce((pairs, arg, i, args) => i % 2 ? pairs : [...pairs, [arg, args[i + 1]]], []))
const signed = '--signing-key' in options
for (const name of ['kernel', 'bootstrap', 'worker', 'relay', signed ? 'signing-key' : 'signing-request', 'source', 'output']) {
  if (!options[`--${name}`]) throw new Error(`missing --${name}`)
}
if (signed && options['--signing-request']) throw new Error('--signing-key and --signing-request are exclusive')
const output = resolve(options['--output'])
await mkdir(output, { recursive: false, mode: 0o755 })
await mkdir(join(output, 'artifacts'))
const key = signed ? createPrivateKey(await readFile(options['--signing-key'])) : null
if (key && key.asymmetricKeyType !== 'ed25519') throw new Error('Ed25519 release signing key required')
const publicKey = key && createPublicKey(key).export({ format: 'der', type: 'spki' }).subarray(-32).toString('base64')
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
  ['chariox-managed-bootstrap.service', await readFile(join(directory, '../deploy/managed-kernel/chariox-vm-bootstrap.service'))],
  ['chariox-build-attestation', attestation],
  ...signed ? [
    ['chariox-build-attestation-signature', Buffer.from(sign(null, attestation, key).toString('base64') + '\n')],
    ['chariox-builder-public-key', Buffer.from(publicKey + '\n')],
  ] : [],
])
const artifacts = []
for (const [name, path] of bootArtifacts) {
  const bytes = contents.get(name)
  if (bytes) await writeFile(join(output, 'artifacts', name), bytes, { mode: 0o644 })
  artifacts.push({ name, path, sha256: bytes ? sha(bytes) : null })
}
if (!signed) {
  const request = signingRequestBytes('boot', { sourceCommit, sourceTree, attestationBytes: attestation, entries: artifacts })
  await writeFile(options['--signing-request'], request, { flag: 'wx' })
  console.log(JSON.stringify({ requestDigest: sha(request), sourceCommit, sourceTree, output }))
} else {
  const manifest = manifestBytes('boot', { sourceCommit, sourceTree }, artifacts)
  await writeFile(join(output, 'release-manifest.json'), manifest)
  await writeFile(join(output, 'release-manifest.sig'), sign(null, manifest, key).toString('base64') + '\n')
  console.log(JSON.stringify({ digest: sha(manifest), sourceCommit, sourceTree, output }))
}
