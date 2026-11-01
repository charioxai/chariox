import assert from 'node:assert/strict'
import { generateKeyPairSync, sign } from 'node:crypto'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { importSignedNative } from './signed-native.mjs'
import { canonical, hash } from './manifest.mjs'

test('MP-08 / MP-09 / MP-10 / MP-11: consume signed native artifacts without a private key on the builder', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cicd-signed-'))
  // Ephemeral in-memory test key only; never an owner or release key.
  const { publicKey, privateKey } = generateKeyPairSync('ed25519')
  const pem = publicKey.export({ type: 'spki', format: 'pem' })
  const fingerprint = hash(publicKey.export({ type: 'spki', format: 'der' }))
  const revision = 'b'.repeat(40), bytes = Buffer.from('synthetic-native-not-a-release')
  const receipt = { version: '0.1.0', revision, files: Object.fromEntries(['chariox', 'chariox-kernel', 'chariox-relay'].map(name => [name, hash(bytes)])),
    bundle_sha256: hash(bytes), relay_sha256: hash(bytes), signing: 'signed' }
  const config = { input: root, output: join(root, 'imported'), version: receipt.version, revision, publicKey: pem, fingerprint }
  try {
    for (const name of ['native.tar.gz', 'chariox', 'chariox-kernel', 'chariox-relay']) await writeFile(join(root, name), bytes)
    await writeFile(join(root, 'native.json'), canonical(receipt))
    await writeFile(join(root, 'native.json.sig'), sign(null, Buffer.from(canonical(receipt)), privateKey))
    await assert.rejects(() => importSignedNative({ ...config, fingerprint: 'c'.repeat(64) }), /public pin/)
    await assert.rejects(() => importSignedNative({ ...config, revision: 'd'.repeat(40) }), /identity/)
    await writeFile(join(root, 'chariox-relay'), 'tampered')
    await assert.rejects(() => importSignedNative(config), /checksum/)
    await writeFile(join(root, 'chariox-relay'), bytes)
    const result = await importSignedNative(config)
    assert.equal(result.signing, 'signed')
    assert.equal(result.inventory_sha256, hash(canonical(receipt)))
    assert.equal(await readFile(join(config.output, 'native.tar.gz'), 'utf8'), bytes.toString())
  } finally { await rm(root, { recursive: true, force: true }) }
})
