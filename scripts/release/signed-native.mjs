// MP-08 / MP-09 / MP-10 / MP-11: only public verification material enters CI.
import { createHash, createPublicKey, verify } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { copyFile, lstat, mkdir, readFile, writeFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { hash, isVersion } from './manifest.mjs'

const binaries = ['chariox', 'chariox-kernel', 'chariox-relay']
async function fileHash(path) {
  const info = await lstat(path)
  if (!info.isFile() || info.isSymbolicLink()) throw Error('signed artifact must be a regular file')
  const digest = createHash('sha256')
  for await (const chunk of createReadStream(path)) digest.update(chunk)
  return digest.digest('hex')
}

export async function verifySignedNative({ input, binaryRoot = input, version, revision, publicKey, fingerprint }) {
  if (typeof publicKey !== 'string' || publicKey.length > 4096 || !publicKey.startsWith('-----BEGIN PUBLIC KEY-----') ||
      !publicKey.trimEnd().endsWith('-----END PUBLIC KEY-----')) throw Error('approved public pin required')
  const key = createPublicKey(publicKey)
  if (key.asymmetricKeyType !== 'ed25519' || !/^[0-9a-f]{64}$/.test(fingerprint) ||
      hash(key.export({ type: 'spki', format: 'der' })) !== fingerprint) throw Error('approved public pin mismatch')
  const path = join(input, 'native.json')
  const metadata = await lstat(path)
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > 16 * 1024) throw Error('bounded signed native inventory required')
  const signatureInfo = await lstat(join(input, 'native.json.sig'))
  if (!signatureInfo.isFile() || signatureInfo.isSymbolicLink() || signatureInfo.size !== 64) throw Error('bounded native inventory signature required')
  const bytes = await readFile(path), signature = await readFile(join(input, 'native.json.sig'))
  if (signature.length !== 64 || !verify(null, bytes, key, signature)) throw Error('native inventory signature rejected')
  const receipt = JSON.parse(bytes)
  if (Object.keys(receipt).sort().join(',') !== 'bundle_sha256,files,relay_sha256,revision,signing,version' ||
      receipt.signing !== 'signed' || !isVersion(version) || receipt.version !== version ||
      !/^[0-9a-f]{40}$/.test(revision) || receipt.revision !== revision ||
      Object.keys(receipt.files ?? {}).sort().join(',') !== [...binaries].sort().join(',')) throw Error('signed native identity mismatch')
  for (const [name, expected] of [...binaries.map(name => [name, receipt.files[name]]), ['native.tar.gz', receipt.bundle_sha256]]) {
    if (!/^[0-9a-f]{64}$/.test(expected) || await fileHash(join(name === 'native.tar.gz' ? input : binaryRoot, name)) !== expected) throw Error('signed native checksum mismatch')
  }
  if (receipt.relay_sha256 !== receipt.files['chariox-relay']) throw Error('signed relay checksum mismatch')
  return { ...receipt, inventory_sha256: hash(bytes) }
}

export async function importSignedNative(options) {
  const receipt = await verifySignedNative(options)
  if (resolve(options.input) === resolve(options.output)) throw Error('signed input and disposable output must differ')
  await mkdir(options.output, { recursive: false })
  for (const name of [...binaries, 'native.tar.gz', 'native.json', 'native.json.sig']) await copyFile(join(options.input, name), join(options.output, name))
  // The Mac handoff may be replaced by another producer; verify the copied bytes too.
  await verifySignedNative({ ...options, input: options.output })
  await writeFile(join(options.output, 'release-public.pem'), options.publicKey, { flag: 'wx' })
  return receipt
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const config = JSON.parse(await readFile(process.argv[2], 'utf8'))
  importSignedNative({ ...config, publicKey: process.env.RELEASE_SIGNING_PUBLIC_KEY, fingerprint: process.env.RELEASE_SIGNING_FINGERPRINT })
    .then(async receipt => {
      const { run } = await import('./process.mjs')
      if (run(join(config.output, 'chariox'), ['--version']) !== `chariox ${config.version}`) throw Error('signed CLI product version differs')
      console.log(JSON.stringify({ revision: receipt.revision, signing: receipt.signing, inventory_sha256: receipt.inventory_sha256 }))
    })
    .catch(() => { console.error('MP-08 / MP-09 / MP-10 / MP-11: signed native input rejected; check exact source/version, public pin and artifact hashes'); process.exitCode = 1 })
}
