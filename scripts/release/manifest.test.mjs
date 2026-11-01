import assert from 'node:assert/strict'
import test from 'node:test'
import { attest, canonical, checks, createManifest, hash, observedPins, promote, validateManifest } from './manifest.mjs'

const sha = 'a'.repeat(64)
const revision = 'b'.repeat(40)
function candidate(version = '0.1.0') {
  return createManifest({ version, kit_sha256: 'a'.repeat(64), signing: { status: 'signed', inventory_sha256: 'a'.repeat(64) }, source: Object.fromEntries(['cloud', 'oss', 'relay'].map(name => [name, { ref: 'refs/heads/main', commit: revision }])), cloud: { commit: revision, image: `ghcr.io/charioxai/chariox-cloud-web@sha256:${sha}`,
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
test('MP-08 / MP-09 / MP-10 / MP-11: unsigned inputs or an unbound kit cannot receive acceptance', () => {
  const unsigned = candidate(); unsigned.signing = { status: 'pending', inventory_sha256: null }
  assert.throws(() => attest(unsigned, receipt(unsigned)), /signed native/)
  const noKit = candidate(); noKit.kit_sha256 = null
  assert.throws(() => attest(noKit, receipt(noKit)), /kit/)
})
test('MP-07 / MP-11: component repositories and public evidence URLs are constrained', () => {
  const value = candidate(); value.cloud.image = `ghcr.io/untrusted/other@sha256:${sha}`
  assert.throws(() => validateManifest(value), /repository/)
  const good = candidate(), r = receipt(good)
  r.evidence = 'https://user:example@example.test/path?auth=example'
  assert.throws(() => attest(good, r), /evidence/)
})
test('MP-07 / MP-11: semantic receipt equality does not depend on JSON key order', () => {
  const value = candidate(), r = receipt(value)
  r.observed = Object.fromEntries(Object.entries(r.observed).reverse())
  assert.equal(attest(value, r).state, 'validated')
})
test('MP-08 / MP-09 / MP-10 / MP-11: validated branch staging is refused by promotion', () => {
  const value = candidate()
  value.source = Object.fromEntries(['cloud', 'oss', 'relay'].map(name => [name, { ref: name === 'cloud' ? 'refs/heads/stagenew/web' : 'refs/heads/main', commit: revision }]))
  const branch = createManifest(value)
  const bytes = canonical(attest(branch, receipt(branch)))
  assert.throws(() => promote(bytes, hash(bytes)), /main release/)
})
test('MP-07 / MP-11: reject invalid SemVer before any image is published', () => {
  for (const version of ['0.01.0', '0.1.0-01', '0.1.0-rc.01', '0.1.0-.', `0.1.0-${'a'.repeat(90)}`]) assert.throws(() => candidate(version), /version/)
  assert.equal(candidate('0.1.0-rc.1').version, '0.1.0-rc.1')
  assert.equal(candidate('0.1.0-rc-01').version, '0.1.0-rc-01')
})
