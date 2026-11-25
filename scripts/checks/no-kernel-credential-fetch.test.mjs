import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './no-kernel-credential-fetch.mjs'

const peer = variants => `pub enum RelayPeerRequest {\n${variants}\n}\n`
const files = (variants = '    CancelLeasedPrompt { session_id: String },\n') => ({ 'apps/kernel/src/transport/relay_peer.rs': peer(variants) })
const planted = (r, label) => {
  const result = r.run(check.name)
  assert.equal(result.status, 1, label)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`), `${label}: names the root fix`)
  return result.text
}

test('a peer protocol with no credential message passes', () => {
  const r = makeRepo(files())
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a launch variant that carries a provider credential is red, with the line', () => {
  const r = makeRepo(files())
  r.write('apps/kernel/src/transport/relay_peer.rs', peer('    CancelLeasedPrompt { session_id: String },\n    SubmitLeasedPrompt {\n        prompt: String,\n        provider_launch_credential: Option<RemoteProviderLaunchCredential>,\n    },'))
  assert.match(planted(r, 'launch'), /relay_peer\.rs:3: relay_peer\.rs RelayPeerRequest::SubmitLeasedPrompt moves a credential between kernels \(RemoteProviderLaunchCredential\)/)
  r.dispose()
})

test('a request that asks another kernel for a Vault value is red', () => {
  const r = makeRepo(files('    ResolveWorkerCredentialSecret { credential_id: String },\n'))
  assert.match(planted(r, 'resolve'), /RelayPeerRequest::ResolveWorkerCredentialSecret moves a credential/)
  r.dispose()
})

test('a response carrying a secret type is red', () => {
  const r = makeRepo({ 'apps/kernel/src/transport/relay_peer.rs': 'pub enum RelayPeerResponse {\n    Fetched { value: RemoteCredentialSecretInput },\n}\n' })
  assert.match(planted(r, 'response'), /RelayPeerResponse::Fetched moves a credential between kernels \(RemoteCredentialSecretInput\)/)
  r.dispose()
})

test('an install-time transfer the check names passes', () => {
  const r = makeRepo(files('    InstallProviderAccountCopy { credential: ProviderAccountCopyCredential },\n    InstallManagedSliceRelayToken { relay_token: RelayManagedSliceToken },\n'))
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('the run-time home Vault and Environment uses row 581 names as exceptions pass', () => {
  const r = makeRepo(files('    ResolveHomeCredentialSecret { credential_id: String },\n    InvokeHomeCredentialTool { credential_id: String },\n    CaptureForHomeVault { capture: CredentialCapture },\n    PullProjectEnvironmentSecret { project_id: String, name: String },\n    ManagedSliceRelayTokenRefreshed { relay_token: RelayManagedSliceToken },\n'))
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a credential request outside the named exceptions is still red', () => {
  const r = makeRepo(files('    FetchHomeVaultSecret { credential_id: String },\n'))
  assert.match(planted(r, 'fetch'), /RelayPeerRequest::FetchHomeVaultSecret moves a credential between kernels/)
  r.dispose()
})

test('comments, strings and a test module do not count', () => {
  const r = makeRepo(files('    // ResolveHomeCredentialSecret { secret: RemoteCredentialSecretInput },\n    Note { text: String },\n'))
  r.write('apps/kernel/src/transport/relay_peer.rs', `${peer('    Note { text: String },\n')}#[cfg(test)]\nmod tests {\n    pub enum RelayPeerRequest { Fake { secret: SecretToken } }\n}\n`)
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a listed variant passes, an unlisted second one is red', () => {
  const variants = '    SubmitLeasedPrompt { c: Option<RemoteProviderLaunchCredential> },\n'
  const r = makeRepo({ ...files(variants), 'scripts/checks/no-kernel-credential-fetch.allow.json': '[{"path":"apps/kernel/src/transport/relay_peer.rs","count":1,"why":"row 577"}]\n' })
  assert.equal(r.run(check.name).status, 0)
  r.write('apps/kernel/src/transport/relay_peer.rs', peer(`${variants}    FetchHomeVault { id: String },\n`))
  assert.match(planted(r, 'second'), /FetchHomeVault/)
  r.dispose()
})

test('a credential type that starts with the marker is red', () => {
  for (const type of ['SecretString', 'Token', 'VaultHandle', 'CredentialBlob']) {
    const r = makeRepo(files(`    Launch { value: Option<${type}> },\n`))
    assert.match(planted(r, type), new RegExp(`RelayPeerRequest::Launch moves a credential between kernels \\(${type}\\)`), type)
    r.dispose()
  }
})
