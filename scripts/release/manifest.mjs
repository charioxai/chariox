// MP-07 / MP-11: public release identities, never runtime configuration.
import { createHash } from 'node:crypto'
import { readFile, writeFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'
import { validateSources, requireMainRelease } from './source.mjs'

const sha256 = /^[0-9a-f]{64}$/
const commit = /^[0-9a-f]{40}$/
const image = /^ghcr\.io\/[a-z0-9][a-z0-9/_.-]*@sha256:[0-9a-f]{64}$/
const version = /^0\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/
export const checks = ['health', 'migrations', 'browser', 'relay', 'providers', 'native', 'rollback']
export const canonical = value => `${JSON.stringify(value, null, 2)}\n`
export const hash = bytes => createHash('sha256').update(bytes).digest('hex')
export function isVersion(value) {
  if (typeof value !== 'string' || value.length > 87 || !version.test(value)) return false
  const prerelease = value.includes('-') ? value.slice(value.indexOf('-') + 1) : ''
  return !prerelease.split('.').some(part => /^\d+$/.test(part) && part.length > 1 && part.startsWith('0'))
}

function requireValue(condition, message) {
  if (!condition) throw Error(message)
}

function keys(value, expected, label) {
  requireValue(value && typeof value === 'object' && !Array.isArray(value), `${label}: expected object`)
  requireValue(Object.keys(value).sort().join(',') === [...expected].sort().join(','), `${label}: unexpected or missing fields`)
}

function artifact(value, fields, label) {
  keys(value, fields, label)
  for (const field of fields) {
    const pattern = field === 'commit' ? commit : field === 'image' ? image : sha256
    requireValue(pattern.test(value[field]), `${label}.${field}: expected immutable identity`)
  }
}

export function validateManifest(value, { validated = false } = {}) {
  keys(value, ['schema', 'version', 'state', 'source', 'cloud', 'oss', 'relay', 'signing', 'kit_sha256', 'validation'], 'manifest')
  requireValue(value.schema === 'chariox.release/v2', 'unsupported release schema')
  requireValue(isVersion(value.version), 'expected pre-1.0 semantic version without v, fitting its image tag')
  requireValue(['candidate', 'validated'].includes(value.state), 'inventory is not deployable')
  artifact(value.cloud, ['commit', 'image', 'services_image_sha256', 'archive_sha256', 'source_sha256'], 'cloud')
  // services_image_sha256 is a registry manifest digest; its repository is fixed.
  artifact(value.oss, ['commit', 'image', 'bundle_sha256'], 'oss')
  artifact(value.relay, ['commit', 'image', 'binary_sha256'], 'relay')
  validateSources(value.source, value)
  requireValue(value.kit_sha256 === null || sha256.test(value.kit_sha256), 'invalid deployment kit hash')
  for (const [name, repository] of [['cloud', 'chariox-cloud-web'], ['oss', 'chariox-oss'], ['relay', 'chariox-relay']]) {
    requireValue(value[name].image.startsWith(`ghcr.io/charioxai/${repository}@sha256:`), `${name}: unexpected component repository`)
  }
  keys(value.signing, ['status', 'inventory_sha256'], 'signing')
  requireValue(['pending', 'signed'].includes(value.signing.status), 'invalid signing status')
  requireValue(value.signing.status === 'pending' ? value.signing.inventory_sha256 === null : sha256.test(value.signing.inventory_sha256), 'invalid public signing inventory hash')
  if (value.state === 'validated') {
    const candidate = { ...value, state: 'candidate', validation: null }
    validateReceipt(value.validation, candidate)
  } else requireValue(value.validation === null, 'candidate must not contain a validation receipt')
  if (validated) requireValue(value.state === 'validated', 'production requires validated staging')
  return value
}

export function servicesImage(manifest) {
  return `ghcr.io/charioxai/chariox-cloud-services@sha256:${manifest.cloud.services_image_sha256}`
}

export function observedPins(manifest) {
  return { web: manifest.cloud.image, services: servicesImage(manifest), oss: manifest.oss.image,
    relay: manifest.relay.image, bundle_sha256: manifest.oss.bundle_sha256, relay_sha256: manifest.relay.binary_sha256 }
}

export function validateObservation(observed, manifest) {
  const expected = observedPins(manifest)
  keys(observed, Object.keys(expected), 'observed')
  requireValue(Object.keys(expected).every(name => observed[name] === expected[name]), 'observed artifact identity mismatch')
}

export function validateReceipt(receipt, candidate) {
  requireValue(sha256.test(candidate.kit_sha256), 'deployment kit must be bound before validation')
  requireValue(candidate.signing.status === 'signed', 'signed native inputs required before validation')
  keys(receipt, ['environment', 'manifest_sha256', 'recorded_at', 'checks', 'observed', 'evidence'], 'validation')
  requireValue(receipt.environment === 'staging', 'validation must come from staging')
  requireValue(receipt.manifest_sha256 === hash(canonical(candidate)), 'validation candidate hash mismatch')
  requireValue(typeof receipt.recorded_at === 'string' && Number.isFinite(Date.parse(receipt.recorded_at)), 'validation timestamp required')
  let evidence
  try { evidence = new URL(receipt.evidence) } catch { throw Error('validation evidence HTTPS URL required') }
  requireValue(evidence.protocol === 'https:' && !evidence.username && !evidence.password && !evidence.search && !evidence.hash, 'evidence URL must be public and contain no credential/query material')
  keys(receipt.checks, checks, 'validation.checks')
  requireValue(checks.every(name => receipt.checks[name] === true), 'live acceptance checks have not all passed')
  validateObservation(receipt.observed, candidate)
  return receipt
}

export function attest(candidate, receipt) {
  validateManifest(candidate)
  requireValue(candidate.state === 'candidate', 'expected a candidate')
  validateReceipt(receipt, candidate)
  return validateManifest({ ...candidate, state: 'validated', validation: receipt }, { validated: true })
}

export function promote(bytes, expectedHash) {
  requireValue(sha256.test(expectedHash) && hash(bytes) === expectedHash, 'approved manifest SHA-256 mismatch')
  validateManifest(JSON.parse(bytes), { validated: true })
  requireMainRelease(JSON.parse(bytes))
  return bytes // byte-for-byte: do not rebuild, reserialize, retag or change version.
}

export function createManifest(input) {
  const { version: releaseVersion, source, cloud, oss, relay, kit_sha256 = null, signing = { status: 'pending', inventory_sha256: null } } = input
  return validateManifest({ schema: 'chariox.release/v2', version: releaseVersion, state: 'candidate', source, cloud, oss, relay, signing, kit_sha256, validation: null })
}

async function main(args) {
  const [command, source, destination, extra] = args
  if (!source) throw Error('usage: manifest.mjs create|validate|attest|promote <input> [output] [receipt|approved-hash]')
  const bytes = await readFile(source, 'utf8')
  if (command === 'validate') {
    validateManifest(JSON.parse(bytes), { validated: destination === '--validated' })
    console.log(`MP-07 / MP-11: valid manifest sha256=${hash(bytes)}`)
    return
  }
  if (!destination || !extra && ['attest', 'promote'].includes(command)) throw Error('missing output or receipt/hash')
  let result
  if (command === 'create') result = canonical(createManifest(JSON.parse(bytes)))
  else if (command === 'attest') result = canonical(attest(JSON.parse(bytes), JSON.parse(await readFile(extra, 'utf8'))))
  else if (command === 'promote') result = promote(bytes, extra)
  else throw Error('unknown manifest command')
  await writeFile(destination, result, { flag: 'wx', mode: 0o644 })
  console.log(`MP-07 / MP-11: ${command} sha256=${hash(result)}`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).catch(() => { console.error('MP-07 / MP-11: release manifest rejected; verify schema, immutable pins and receipt'); process.exitCode = 1 })
}
