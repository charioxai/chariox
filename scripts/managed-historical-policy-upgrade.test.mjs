// MP-03/MP-07/MP-10/MP-11: real signed H/I artifacts and their unchanged updater,
// synthetic target signing keys held only in memory, fake host services/health.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { chmod, chown, readFile, readlink, stat } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import test from 'node:test'
import { makeHarness } from './lib/managed-kernel-upgrade-fixture.mjs'

const roots = [ ['H', process.env.MP07_SIGNED_H], ['I', process.env.MP07_SIGNED_I] ]
// Optional actual K-A/K-B payloads, signed with synthetic test keys (public pins only).
async function actualRelease(prefix) {
  const rootfs = process.env[prefix + '_ROOTFS']
  if (!rootfs) return null
  const bytes = await readFile(join(rootfs, 'usr/lib/chariox/release-manifest.json'))
  return { rootfs, digest: 'sha256:' + createHash('sha256').update(bytes).digest('hex'),
    releasePin: process.env[prefix + '_RELEASE_PIN'], builderPin: process.env[prefix + '_BUILDER_PIN'] }
}
const actualKA = await actualRelease('MP07_K_A'), actualKB = await actualRelease('MP07_K_B')
for (const [round, rootfs] of roots) {
  test(`MP-07 signed ${round} actual verifier and updater admit K-A bridge`, {skip:!rootfs}, async context => {
    const manifestPath = join(rootfs, 'usr/lib/chariox/release-manifest.json')
    const bytes = await readFile(manifestPath)
    const installedRelease = {rootfs, digest:'sha256:'+createHash('sha256').update(bytes).digest('hex'),
      releasePin:process.env.MP07_RELEASE_PIN, builderPin:process.env.MP07_BUILDER_PIN}
    const updaterPath = join(rootfs, 'usr/lib/chariox/slice-build-context/deploy/managed-kernel/upgrade-image.sh')
    const protocol = spawnSync(join(rootfs, 'usr/local/bin/chariox-kernel'), ['--print-local-daemon-protocol-version'], {encoding:'utf8'})
    assert.equal(protocol.status,0,protocol.stderr)
    const h = await makeHarness(context, {installedRelease, updaterPath, updaterShell:true, path1Release:true,
      currentProtocol:Number(protocol.stdout), targetProtocol:Number(protocol.stdout),
      targetServicePolicy: Number(process.env.MP07_CANDIDATE_SERVICE_POLICY ?? 1), targetRelease: actualKA})
    await chmod(h.root, 0o755)
    await chown(dirname(h.receiptPath), h.charioxIdentity.uid, h.charioxIdentity.gid)
    await chmod(dirname(h.receiptPath),0o700)
    await chown(h.receiptPath,h.charioxIdentity.uid,h.charioxIdentity.gid)
    await chmod(h.receiptPath,0o600)
    const before = await stat(h.receiptPath)
    const result = h.run({CHARIOX_MANAGED_PROVIDER_TOPOLOGY:'path1',
      CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY:h.trustedBuilderKey,
      CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY:h.nextTrustedBuilderKey})
    assert.equal(result.status,0,result.stderr)
    assert.equal(await readlink(join(h.installRoot,'usr/lib/chariox/current')), 'releases/'+h.target.digest.slice(7))
    const after = await stat(h.receiptPath)
    assert.equal(after.uid,before.uid)
    assert.equal(after.mode & 0o777, before.mode & 0o777)
    // Continue on the same installed host with K-A's versioned upgrade policy.
    const bridgeRelease = {...h.target, releasePin:h.nextTrustedReleaseKey, builderPin:h.nextTrustedBuilderKey}
    const b = await makeHarness(context, {installedRelease:bridgeRelease, path1Release:true,
      currentProtocol:Number(protocol.stdout), targetProtocol:Number(protocol.stdout), targetServicePolicy:2, targetRelease: actualKB})
    // The second hop runs K-A's own updater: the actual payload's copy when supplied.
    const kScript = actualKA ? join(actualKA.rootfs, 'usr/lib/chariox/slice-build-context/deploy/managed-kernel/upgrade-image.sh')
      : new URL('../deploy/managed-kernel/upgrade-image.sh', import.meta.url).pathname
    const second = spawnSync(kScript, [b.target.rootfs, h.target.digest, b.target.digest,
      h.nextTrustedReleaseKey, b.nextTrustedReleaseKey], {encoding:'utf8', env:{...h.env,
        CHARIOX_MANAGED_PROVIDER_TOPOLOGY:'path1',
        CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY:h.nextTrustedBuilderKey,
        CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY:b.nextTrustedBuilderKey}})
    assert.equal(second.status,0,second.stderr)
    assert.equal(await readlink(join(h.installRoot,'usr/lib/chariox/current')), 'releases/'+b.target.digest.slice(7))
    const privateState = await stat(h.receiptPath)
    assert.equal(privateState.uid,0)
    assert.equal(privateState.mode & 0o777,0o600)
    assert.equal((await stat(dirname(h.receiptPath))).mode & 0o777,0o700)
    console.log(`MP-07 ${round} source=${JSON.parse(bytes).sourceCommit} digest=${installedRelease.digest} updater=unchanged path=${round}→K-A→K-B state=root-private targets=${actualKA ? actualKA.digest + '→' + actualKB.digest : 'fixture'}`)
  })
}

// MP-03/MP-07: a newer updater must restore access before an old target starts.
for (const [round, rootfs] of roots) {
  test(`MP-03 K policy 2 downgrade to actual signed ${round} preserves pre-helper access`, {skip:!rootfs || !!process.env.MP07_CANDIDATE_SERVICE_POLICY}, async context => {
    const manifest = await readFile(join(rootfs,'usr/lib/chariox/release-manifest.json'))
    const digest = 'sha256:'+createHash('sha256').update(manifest).digest('hex')
    const protocol = Number(spawnSync(join(rootfs,'usr/local/bin/chariox-kernel'),['--print-local-daemon-protocol-version'],{encoding:'utf8'}).stdout)
    const h = await makeHarness(context,{path1Release:true,currentServicePolicy:2,currentProtocol:protocol,targetProtocol:protocol})
    await chmod(dirname(h.receiptPath),0o700)
    await chmod(h.receiptPath,0o600)
    const result = h.run({CHARIOX_MANAGED_PROVIDER_TOPOLOGY:'path1',
      CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY:h.trustedBuilderKey,
      CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY:process.env.MP07_BUILDER_PIN},
      [rootfs,h.current.digest,digest,h.trustedKey,process.env.MP07_RELEASE_PIN])
    assert.equal(result.status,0,result.stderr)
    assert.equal((await stat(h.receiptPath)).uid,h.charioxIdentity.uid)
    assert.equal((await stat(dirname(h.receiptPath))).uid,h.charioxIdentity.uid)
  })
}
