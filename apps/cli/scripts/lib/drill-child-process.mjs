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
  /model .* is not supported .* account/i,
  /not supported when using .* account/i,
]

const PROVIDER_AUTH_PATTERNS = [
  /\b(?:http|status(?: code)?|response|token refresh failed:?)\s*401\b/i,
  /\b401\s+unauthori[sz]ed\b/i,
  /unauthori[sz]ed/i,
  /authentication/i,
  /not logged in/i,
  /login expired/i,
  /login required/i,
  /token refresh failed/i,
]

const SLICE_AUTH_PATTERNS = [
  /slice_lifecycle\.provider_auth_issues/i,
  /slice .*provider auth/i,
  /provider auth .*slice/i,
  /slice .*not_configured/i,
]

const SLICE_RUNTIME_PATTERNS = [
  /slice_lifecycle\.(?:launch_failed|launch_timeout|container_failed|stuck)/i,
  /slice .*failed to launch/i,
  /slice .*launch timed out/i,
  /slice .*stuck (?:launching|starting)/i,
  /slice .*did not become ready/i,
]

const DOCKER_RUNTIME_PATTERNS = [
  /\bdocker\b.*(?:not found|not running|daemon|cannot connect)/i,
  /\bcolima\b/i,
]

const CLOUD_RUNTIME_PATTERNS = [
  /\bchariox-cloud\b/i,
  /\bOpenShip\b/i,
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

const RELAY_TARGET_FRESHNESS_PATTERNS = [
  /selected relay kernel target is stale/i,
  /relay target .*heartbeat .*stale/i,
  /relay target .*no heartbeat received/i,
  /last heartbeat \d+s ago/i,
]

const REMOTE_WORKER_VERSION_PATTERNS = [
  /remote worker [`'"][^`'"]+[`'"] uses relay peer protocol \d+, but this home kernel requires \d+/i,
  /remote worker checkout [`'"][^`'"]+[`'"] is at commit [0-9a-f]+, but home checkout expects [0-9a-f]+/i,
  /worker .*relay peer protocol .*home kernel requires/i,
  /peer protocol .*version mismatch/i,
]

const REMOTE_HOST_CAPACITY_PATTERNS = [
  /\bENOSPC\b/i,
  /\bno space left on device\b/i,
  /\bdisk full\b/i,
  /remote .*filesystem .*full/i,
  /remote .*needs more free space/i,
]

const TEST_HARNESS_PATTERNS = [
  /spawn (?:cargo|tar|openssl|pnpm|bun|script|screen) ENOENT/i,
  /missing built binary/i,
  /run cargo build/i,
  /missing built CLI/i,
  /docker build .* exited with code/i,
  /couldn't read .*examples\/workflow-code/i,
]

const RUNTIME_TIMEOUT_PATTERNS = [
  /timed out waiting for (?:provider run|agents? to become idle|agent idle|file content|\/|TCP listener)/i,
  /did not become ready/i,
  /did not expose runtime MCP binding/i,
  /provider run ended before ready/i,
  /last_observation=/i,
]

const PROVIDER_RUNTIME_PATTERNS = [
  /timed out waiting for marker THREAD_TRANSFER_[A-Z0-9_]+/i,
  /timed out waiting for marker .*; ordered_match=/i,
  /provider thread transfer drill failed/i,
]

const KERNEL_AUTHORITY_PATTERNS = [
  /agent [`'"][^`'"]+[`'"] does not belong to session/i,
  /duplicate_chariox_agent_bindings/i,
  /multi_interface_agent_bindings/i,
  /duplicate .*provider run .*bindings?/i,
  /multiple .*provider runs? .*bound .*agent/i,
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

const RUNTIME_PROJECTION_HEALTH_PATTERNS = [
  /runtime[-_ ]projection[-_ ]health/i,
  /daemon health .*projection/i,
  /kernel .*projection .*invariant/i,
  /projection invariant .*failed/i,
  /read[- ]model .*stale/i,
  /runtime .*projection .*stale/i,
  /runtime .*projection .*reconciliation .*failed/i,
]

const PROJECTION_STALENESS_PATTERNS = [
  /projection .*stale/i,
  /stale .*projection/i,
  /projection .*reconciliation .*failed/i,
]

const WORKER_EXECUTION_PATTERNS = [
  /worker kernel .*failed/i,
  /remote worker .*failed/i,
  /leased agent .*failed (?:to launch|to start|during execution)/i,
  /remote lease .*provider run .*failed/i,
  /worker execution failed/i,
]

const UI_CLIENT_PROJECTION_PATTERNS = [
  /ui\/client projection/i,
  /client projection .*failed/i,
  /web terminal .*projection/i,
  /tui .*projection/i,
  /terminal event .*not .*rendered/i,
  /transcript .*render(?:ing)? .*failed/i,
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
  if (SLICE_RUNTIME_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'slice-runtime'
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
  if (RELAY_TARGET_FRESHNESS_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'relay-target-freshness'
  }
  if (REMOTE_WORKER_VERSION_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'remote-worker-version'
  }
  if (REMOTE_HOST_CAPACITY_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'remote-host-capacity'
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
  if (RUNTIME_PROJECTION_HEALTH_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'runtime-projection-health'
  }
  if (PROJECTION_STALENESS_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'projection-staleness'
  }
  if (WORKER_EXECUTION_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'worker-execution'
  }
  if (UI_CLIENT_PROJECTION_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'ui-client-projection'
  }
  if (WORKSPACE_LIVE_SYNC_CONFLICT_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'workspace-live-sync-conflict'
  }
  if (KERNEL_AUTHORITY_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'kernel-authority'
  }
  if (PROVIDER_RUNTIME_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'provider-error'
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
    classification === 'relay-target-freshness'
      ? 'Relay target heartbeat freshness blocked validation before the runtime behavior could be proven.'
      : null,
    classification === 'remote-worker-version'
      ? 'Remote worker kernel version/protocol skew blocked validation before the runtime behavior could be proven.'
      : null,
    classification === 'remote-host-capacity'
      ? 'Remote host disk or filesystem capacity blocked validation before the runtime behavior could be proven.'
      : null,
    classification === 'test-harness'
      ? 'Local drill prerequisites or build tooling blocked validation before the runtime behavior could be proven.'
      : null,
    classification === 'runtime-timeout'
      ? 'Runtime state did not converge before the drill timeout.'
      : null,
    classification === 'provider-error'
      ? 'Provider runtime did not complete the drill before validation timeout; inspect preserved provider logs and history.'
      : null,
    classification === 'remote-extension-sync'
      ? 'Remote extension manifest state did not reconcile between home and worker kernels.'
      : null,
    classification === 'runtime-projection-health'
      ? 'Runtime projection health failed; inspect kernel read-model freshness, invariant drift, and reconciliation events.'
      : null,
    classification === 'projection-staleness'
      ? 'Kernel projection state is stale; inspect projection health, read-model freshness, and reconciliation events.'
      : null,
    classification === 'worker-execution'
      ? 'Remote worker execution failed; inspect worker kernel logs, leased-agent state, and preserved worker artifacts.'
      : null,
    classification === 'ui-client-projection'
      ? 'UI/client projection failed; inspect web/TUI terminal projection logs, transcript rendering state, and preserved captures.'
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
    classification === 'slice-runtime'
      ? 'Slice runtime failed; inspect slice lifecycle events, container logs, and worker kernel state.'
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
