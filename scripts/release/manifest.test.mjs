import assert from 'node:assert/strict'
import test from 'node:test'
import { attest, canonical, checks, createManifest, hash, observedPins, promote, validateManifest } from './manifest.mjs'

const sha = 'a'.repeat(64)
const revision = 'b'.repeat(40)
function candidate() {
  return createManifest({ version: '0.1.0', cloud: { commit: revision, image: `ghcr.io/charioxai/chariox-cloud-web@sha256:${sha}`,
    services_image_sha256: sha, archive_sha256: sha, source_sha256: sha },
  oss: { commit: revision, image: `ghcr.io/charioxai/chariox-oss@sha256:${sha}`, bundle_sha256: sha },
  relay: { commit: revision, image: `ghcr.io/charioxai/chariox-relay@sha256:${sha}`, binary_sha256: sha } })
}
function receipt(value) {
  return { environment: 'staging', manifest_sha256: hash(canonical(value)), recorded_at: '2026-10-24T00:00:00Z',
    checks: Object.fromEntries(checks.map(name => [name, true])), observed: observedPins(value), evidence: 'https://example.test/real-evidence' }
}
test('MP-07 / MP-11: reject unresolved inventories, tags and image config IDs', () => {
  for (const image of ['ghcr.io/charioxai/web:latest', `sha256:${sha}`, null]) {
    const value = candidate(); value.cloud.image = image
    assert.throws(() => validateManifest(value), /immutable/)
  }
  const value = candidate(); value.state = 'inventory'
  assert.throws(() => validateManifest(value), /not deployable/)
})
test('MP-07 / MP-11: reject forged, incomplete and wrong-candidate live receipts', () => {
  const value = candidate()
  for (const change of [r => { r.manifest_sha256 = 'c'.repeat(64) }, r => { r.checks.browser = false },
    r => { r.observed.relay = 'ghcr.io/charioxai/relay:latest' }, r => { r.environment = 'local' }]) {
    const r = receipt(value); change(r)
    assert.throws(() => attest(value, r))
  }
  assert.throws(() => promote(canonical(value), hash(canonical(value))), /validated staging/)
})
test('MP-07 / MP-11: promotion and rollback preserve all staged bytes', () => {
  const value = candidate(), validated = attest(value, receipt(value)), bytes = canonical(validated)
  assert.equal(promote(bytes, hash(bytes)), bytes)
  assert.throws(() => promote(bytes, 'c'.repeat(64)), /SHA-256 mismatch/)
  const tampered = JSON.parse(bytes); tampered.oss.bundle_sha256 = 'c'.repeat(64)
  assert.throws(() => validateManifest(tampered), /hash mismatch/)
})
test('MP-07 / MP-11: reject unexpected fields that could carry runtime secrets', () => {
  const value = candidate(); value.environment = {}
  assert.throws(() => validateManifest(value), /unexpected/)
})
