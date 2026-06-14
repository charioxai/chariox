import { spawn } from 'node:child_process'
import { redactDrillSecretText } from './drill-secrets.mjs'

const PROVIDER_ACCOUNT_PATTERNS = [
  /insufficient balance/i,
  /no credits?/i,
  /manage your billing/i,
  /billing (hard )?limit/i,
  /insufficient_quota/i,
  /rate limit/i,
  /usage limit/i,
]

const PROVIDER_AUTH_PATTERNS = [
  /\b(?:http|status(?: code)?|response|token refresh failed:?)\s*401\b/i,
  /\b401\s+unauthori[sz]ed\b/i,
  /unauthori[sz]ed/i,
  /authentication/i,
  /not logged in/i,
  /login required/i,
  /token refresh failed/i,
]

const SLICE_AUTH_PATTERNS = [
  /slice_lifecycle\.provider_auth_issues/i,
  /slice .*provider auth/i,
  /provider auth .*slice/i,
  /slice .*not_configured/i,
]

const DOCKER_RUNTIME_PATTERNS = [
  /\bdocker\b.*(?:not found|not running|daemon|cannot connect)/i,
  /\bcolima\b/i,
]

const CLOUD_RUNTIME_PATTERNS = [
  /\barroba-cloud\b/i,
  /\bScalingo\b/i,
  /cloud .*deployment/i,
  /deployment .*did not become ready/i,
  /publication deployment/i,
  /\b(?:502|503|504)\b.*(?:cloud|deployment|gateway|service)/i,
]

const RELAY_RUNTIME_PATTERNS = [
  /relay target .*not .*reachable/i,
  /target daemon disconnected from relay/i,
  /timed out waiting for relay/i,
  /relay read failed or ended/i,
  /websocket protocol error/i,
  /connection reset/i,
  /target.*(?:stale|offline)/i,
]

const TEST_HARNESS_PATTERNS = [
  /spawn (?:cargo|tar|openssl|pnpm|bun|script|screen) ENOENT/i,
  /missing built binary/i,
  /run cargo build/i,
  /missing built CLI/i,
]

const RUNTIME_TIMEOUT_PATTERNS = [
  /timed out waiting for (?:provider run|agents? to become idle|agent idle|file content|\/|TCP listener)/i,
  /did not become ready/i,
  /did not expose runtime MCP binding/i,
  /provider run ended before ready/i,
  /last_observation=/i,
]

const KERNEL_AUTHORITY_PATTERNS = [
  /agent [`'"][^`'"]+[`'"] does not belong to session/i,
  /wrong (?:leased agent|lease|provider run|collab user|session|agent)/i,
  /forged .*manifest/i,
  /stale .*manifest/i,
  /revoked grant/i,
  /authority .*drift/i,
]

const REMOTE_EXTENSION_SYNC_PATTERNS = [
  /remote extension manifest sync (?:failed|returned an unexpected response)/i,
  /remote_extension_manifest_sync/i,
  /home_extension\.manifest\.(?:failed|stale)/i,
  /pending revoke/i,
]

const WORKSPACE_LIVE_SYNC_CONFLICT_PATTERNS = [
  /workspace live sync .*conflict/i,
  /skipped_conflict/i,
  /changed outside workspace live sync/i,
  /live sync conflict/i,
]

export function classifyDrillChildFailure(text) {
  if (PROVIDER_ACCOUNT_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'provider-account'
  }
  if (SLICE_AUTH_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'slice-auth'
  }
  if (PROVIDER_AUTH_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'provider-auth'
  }
  if (DOCKER_RUNTIME_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'docker-runtime'
  }
  if (CLOUD_RUNTIME_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'cloud-runtime'
  }
  if (RELAY_RUNTIME_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'relay-runtime'
  }
  if (TEST_HARNESS_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'test-harness'
  }
  if (REMOTE_EXTENSION_SYNC_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'remote-extension-sync'
  }
  if (WORKSPACE_LIVE_SYNC_CONFLICT_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'workspace-live-sync-conflict'
  }
  if (KERNEL_AUTHORITY_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'kernel-authority'
  }
  if (RUNTIME_TIMEOUT_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'runtime-timeout'
  }
  if (/provider_error|OpenCode error|Codex error|Claude error/i.test(text)) {
    return 'provider-error'
  }
  return 'child-process'
}

export function formatDrillChildFailure(label, code, signal, stdout, stderr) {
  const combined = `${stdout}\n${stderr}`.trim()
  const classification = classifyDrillChildFailure(combined)
  const exit = signal ? `signal ${signal}` : `code ${code}`
  const tail = redactDrillSecretText(combined.split('\n').slice(-40).join('\n').trim())
  return [
    `${label} child failed with ${exit} (${classification})`,
    classification === 'provider-account'
      ? 'Provider account/billing blocked validation before the remote runtime behavior could be proven.'
      : null,
    classification === 'provider-auth'
      ? 'Provider authentication blocked validation before the remote runtime behavior could be proven.'
      : null,
    classification === 'docker-runtime'
      ? 'Docker or Colima blocked validation before the runtime behavior could be proven.'
      : null,
    classification === 'cloud-runtime'
      ? 'Cloud control-plane or deployment infrastructure blocked validation before the runtime behavior could be proven.'
      : null,
    classification === 'relay-runtime'
      ? 'Relay transport blocked validation before the runtime behavior could be proven.'
      : null,
    classification === 'test-harness'
      ? 'Local drill prerequisites or build tooling blocked validation before the runtime behavior could be proven.'
      : null,
    classification === 'runtime-timeout'
      ? 'Runtime state did not converge before the drill timeout.'
      : null,
    classification === 'remote-extension-sync'
      ? 'Remote extension manifest state did not reconcile between home and worker kernels.'
      : null,
    classification === 'workspace-live-sync-conflict'
      ? 'Workspace Live Sync detected a conflict or external change that needs reconciliation.'
      : null,
    classification === 'kernel-authority'
      ? 'Kernel authority state rejected the request; inspect session, agent, lease, and provider-run bindings.'
      : null,
    classification === 'slice-auth'
      ? 'Slice provider authentication is missing or configured for the wrong account.'
      : null,
    tail ? `child output tail:\n${tail}` : null,
  ].filter(Boolean).join('\n')
}

export async function runNodeDrillChild(args, cwd, { label }) {
  return await new Promise((resolve, reject) => {
    const child = spawn('node', args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => {
      process.stdout.write(chunk)
      stdout += chunk.toString()
    })
    child.stderr.on('data', (chunk) => {
      process.stderr.write(chunk)
      stderr += chunk.toString()
    })
    child.on('exit', (code, signal) => {
      if (code === 0) resolve(stdout)
      else reject(new Error(formatDrillChildFailure(label, code, signal, stdout, stderr)))
    })
    child.on('error', reject)
  })
}
