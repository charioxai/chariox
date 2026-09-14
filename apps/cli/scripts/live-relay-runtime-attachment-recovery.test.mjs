import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

import {
  makeChildrenEnv,
  parseArgs,
} from "./live-relay-runtime-drill.mjs"
import {
  ATTACHMENT_RECOVERY_PUBLIC_REQUEST_COUNT,
  ATTACHMENT_RECOVERY_PUBLIC_REQUEST_LABELS,
  createAttachmentRecoveryLedger,
} from "../../../scripts/candidate-kernel-attachment-recovery-helpers.mjs"

const drillSourceUrl = new URL("./live-relay-runtime-drill.mjs", import.meta.url)

function decodeJwtPayload(token) {
  return JSON.parse(Buffer.from(token.split(".")[1], "base64url").toString("utf8"))
}

test("relay attachment recovery mode requires staged binaries and uses its bounded default", () => {
  const options = parseArgs([
    "--attachment-recovery",
    "--relay-binary",
    "/staged/chariox-relay",
    "--kernel-binary",
    "/staged/chariox-kernel",
    "--expected-protocol",
    "326",
  ])
  assert.equal(options.attachmentRecovery, true)
  assert.equal(options.relayBinary, "/staged/chariox-relay")
  assert.equal(options.kernelBinary, "/staged/chariox-kernel")
  assert.equal(options.expectedProtocol, 326)
  assert.equal(options.timeoutMs, 60_000)
  assert.throws(
    () => parseArgs(["--attachment-recovery", "--relay-binary"]),
    /--relay-binary requires a value/,
  )
})

test("existing relay fixture issues scoped client and daemon auth for one isolated target", () => {
  const rootDir = "/tmp/chariox-relay-attachment-recovery-fixture"
  const ports = {
    relayPort: 45168,
    kernelPort: 46168,
    mcpPort: 46169,
    openCodePort: 47168,
    codexPort: 47169,
  }
  const envs = makeChildrenEnv(ports, rootDir)
  const clientClaims = decodeJwtPayload(envs.relayToken)
  const daemonClaims = decodeJwtPayload(envs.daemonEnv.CHARIOX_RELAY_TOKEN)

  assert.ok(clientClaims.allowed_actions.includes("client.connect"))
  assert.ok(clientClaims.allowed_actions.includes("packet.route"))
  assert.equal(clientClaims.subject_kind, "client")
  assert.equal(daemonClaims.subject_kind, "kernel")
  assert.notEqual(envs.relayToken, envs.daemonEnv.CHARIOX_RELAY_TOKEN)
  assert.equal(envs.daemonEnv.CHARIOX_RELAY_URL, "ws://127.0.0.1:45168")
  assert.equal(envs.daemonEnv.CHARIOX_HOME, `${rootDir}/home`)
  assert.equal(envs.relayEnv.CHARIOX_RELAY_SCOPED_ISSUER, "chariox-relay-runtime-drill")
  assert.equal(envs.relayEnv.CHARIOX_RELAY_SCOPED_HMAC_SECRET, "chariox-relay-runtime-drill-secret")
})

test("relay daemon fixture binds token machine identity to kernel registration", () => {
  const rootDir = "/tmp/chariox-relay-attachment-recovery-machine-identity-fixture"
  const ports = {
    relayPort: 45178,
    kernelPort: 46178,
    mcpPort: 46179,
    openCodePort: 47178,
    codexPort: 47179,
  }
  const envs = makeChildrenEnv(ports, rootDir)
  const daemonClaims = decodeJwtPayload(envs.daemonEnv.CHARIOX_RELAY_TOKEN)

  assert.equal(daemonClaims.machine_id, envs.daemonEnv.CHARIOX_MACHINE_ID)
  assert.match(daemonClaims.machine_id, /^relay-drill-machine-/)
  assert.notEqual(daemonClaims.machine_id, envs.daemonEnv.CHARIOX_DAEMON_ID)
})

test("runnable mode stays on existing encrypted relay fixtures and production recovery wiring", async () => {
  const source = await readFile(drillSourceUrl, "utf8")
  const modeStart = source.indexOf("async function runAttachmentRecoveryScenario")
  const modeEnd = source.indexOf("async function main")
  assert.ok(modeStart >= 0)
  assert.ok(modeEnd > modeStart)
  const mode = source.slice(modeStart, modeEnd)

  assert.match(source, /loadCliModules\(cliRuntimeDir\)/)
  assert.match(source, /makeChildrenEnv\(ports, rootDir\)/)
  assert.match(source, /waitForLocalDaemon\(/)
  assert.match(source, /waitForRelayTarget\(/)
  assert.match(source, /spawnProcess\(/)
  assert.match(source, /claimExactChildProcess/)
  assert.match(source, /stopOwnedChild/)
  assert.match(mode, /new LocalIpcClient\(relayUrl, clientOptions\)/)
  assert.match(mode, /new LocalIpcClient\(kernelUrl\)/)
  assert.match(mode, /createSessionRequest\(workspace, workspace, 'candidate-relay-attachment-recovery'\)/)
  assert.match(mode, /relayAuthToken: envs\.relayToken/)
  assert.match(mode, /targetDaemonAlias: envs\.daemonAlias/)
  assert.match(mode, /clientA\.subscribeToKernelEvents/)
  assert.match(mode, /clientB\.subscribeToKernelEvents/)
  assert.match(mode, /createKernelRestartRecoveryController/)
  assert.match(mode, /assertStaleAttachmentRejected/)
  assert.match(mode, /recoveryDisconnected = true/)
  assert.match(mode, /recoveryController\.recover\(\)/)
  assert.match(source, /assertNoSubmitPromptRequest/)
  assert.doesNotMatch(mode, /launchProviderRunRequest\(/)
  assert.doesNotMatch(mode, /submitPromptRequest\(/)
  assert.doesNotMatch(mode, /succeeded = true\s+return/)
  assert.match(source, /kernel-restart-recovery-controller\.js/)
  assert.match(source, /runVersionProbe/)
  assert.match(source, /if \(failure\) throw failure/)
})

test("relay attachment recovery keeps the existing exact public-request contract", async () => {
  const source = await readFile(drillSourceUrl, "utf8")
  const ledger = createAttachmentRecoveryLedger()
  for (const label of ATTACHMENT_RECOVERY_PUBLIC_REQUEST_LABELS) ledger.record(label)
  assert.equal(ledger.assertComplete(), ATTACHMENT_RECOVERY_PUBLIC_REQUEST_COUNT)
  for (const label of ATTACHMENT_RECOVERY_PUBLIC_REQUEST_LABELS) {
    const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
    assert.match(source, new RegExp(`ledger\\.record\\('${escaped}'\\)`), `relay mode must count ${label}`)
  }
  assert.match(source, /successfulRequests: ledger\.count/)
  assert.match(source, /if \(options\.attachmentRecovery && succeeded && !failure/)
})
