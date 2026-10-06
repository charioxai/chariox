import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import { access, mkdir, mkdtemp } from "node:fs/promises"
import { createServer } from "node:http"
import { homedir } from "node:os"
import { join, resolve } from "node:path"
import { setTimeout as sleep } from "node:timers/promises"
import { LocalIpcClient } from "../../../packages/kernel-client/dist/ipc.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "../../../packages/kernel-client/dist/kernel-types.js"
import { cloudRelayStatusRequest, pollCloudRelayLoginRequest, relayStatusRequest } from "../../../packages/kernel-client/dist/ipc-requests.js"
import { pollCloudDeviceLogin } from "../dist/cloud-relay.js"
import { pollCloudRelayLogin, startCloudRelayLogin } from "../dist/relay-api.js"
import { startHostedCloudLink } from "../dist/cloud-command-lifecycle.js"

// Uses a fresh private kernel and synthetic HTTP responses; no hosted account,
// provider, relay or browser credentials are involved. Retain its private home
// outside the repository/evidence so generated kernel identity keys stay private.
const binary = process.env.CHARIOX_KERNEL_BINARY
assert.ok(binary, "Set CHARIOX_KERNEL_BINARY to the locally built chariox-kernel")
await access(binary)
const homes = join(homedir(), ".chariox", "dev", "device-consent-denial")
await mkdir(homes, { recursive: true, mode: 0o700 })
const privateHome = await mkdtemp(join(homes, "kernel-"))
const server = createServer()
server.listen(0, "127.0.0.1")
await once(server, "listening")
const apiUrl = `http://127.0.0.1:${server.address().port}`
let polls = 0
const advertised = []
let forceUnadvertisedDenial = false
let legacyServer = false
let legacyPolls = 0
let rejectedPoll
const legacyRequests = []
server.on("request", async (request, response) => {
  try {
    assert.equal(request.method, "POST")
    let raw = ""
    for await (const chunk of request) raw += chunk
    let result
    if (request.url === "/auth/device/start") {
      result = { deviceCode: "synthetic-device-code", userCode: "ABCD-EFGH",
        verificationUrl: `${apiUrl}/activate?user_code=ABCD-EFGH`,
        expiresAt: new Date(Date.now() + 30_000).toISOString(), intervalSeconds: 1 }
    } else {
      assert.equal(request.url, "/auth/device/poll")
      const body = JSON.parse(raw)
      assert.equal(body.deviceCode, "synthetic-device-code")
      assert.ok(body.supportsAccessDenied === undefined || typeof body.supportsAccessDenied === "boolean")
      if (legacyServer || rejectedPoll) {
        legacyRequests.push(body)
        if (rejectedPoll || "supportsAccessDenied" in body) {
          const rejection = rejectedPoll ?? { status: 400, code: "invalid_request", message: "Request validation failed" }
          response.writeHead(rejection.status, { "Content-Type": "application/json" })
          response.end(JSON.stringify({ error: { code: rejection.code, message: rejection.message } }))
          return
        }
        assert.deepEqual(body, { deviceCode: "synthetic-device-code" })
        legacyPolls += 1
        result = legacyPolls % 2 === 1
          ? { status: "authorization_pending", intervalSeconds: 1, expiresAt: "2030-01-01T00:00:00Z" }
          : { status: "approved", profile: {
              email: "fixture@example.invalid", accountId: "fixture-account", userId: "fixture-user",
              accountSlug: "fixture", realmId: "fixture-realm", relayUrl: "wss://relay.example.invalid",
              issuerId: "fixture-issuer", machineId: "fixture-machine", machineAlias: "Legacy server drill",
            }, cloudSessionToken: "synthetic-cloud-session", cloudSessionExpiresAt: "2030-01-01T00:00:00Z" }
      } else {
        advertised.push(body.supportsAccessDenied === true)
        polls += 1
        result = { status: (body.supportsAccessDenied === true || forceUnadvertisedDenial) ? "access_denied" : "expired_token" }
      }
    }
    response.writeHead(200, { "Content-Type": "application/json" })
    response.end(JSON.stringify(result))
  } catch (error) {
    response.writeHead(500)
    response.end(String(error))
  }
})
// Reserve an available loopback port before starting the isolated kernel.
const reservation = createServer()
reservation.listen(0, "127.0.0.1")
await once(reservation, "listening")
const port = reservation.address().port
const mcpReservation = createServer()
mcpReservation.listen(0, "127.0.0.1")
await once(mcpReservation, "listening")
const mcpPort = mcpReservation.address().port
await Promise.all([reservation, mcpReservation].map((listener) =>
  new Promise((resolve) => listener.close(resolve))))
const kernelEnv = {
  PATH: process.env.PATH, HOME: process.env.HOME,
  CHARIOX_HOME: privateHome, CHARIOX_KERNEL_HOST: "127.0.0.1",
  CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_HOST: "127.0.0.1", CHARIOX_MCP_PORT: String(mcpPort),
  CHARIOX_MACHINE_ALIAS: "Denial drill machine",
}
const kernel = spawn(resolve(binary), [], { cwd: privateHome, env: kernelEnv, stdio: ["ignore", "ignore", "pipe"] })
let kernelError = ""
kernel.stderr.on("data", (chunk) => { kernelError = (kernelError + chunk).slice(-8192) })
kernel.on("error", (error) => { kernelError = String(error) })
let client
try {
  const endpoint = `ws://127.0.0.1:${port}/kernel`
  let ready = false
  for (let attempt = 0; attempt < 80; attempt += 1) {
    assert.ok(kernel.exitCode === null && kernel.signalCode === null, `isolated kernel exited before readiness: ${kernelError}`)
    const probe = new LocalIpcClient(endpoint, { localAuthEnvironment: kernelEnv, controlRequestRetryDeadlineMs: 1000, controlResponseStallMs: 1000 })
    try {
      await Promise.race([
        probe.send(relayStatusRequest()),
        sleep(500, undefined, { ref: false }).then(() => { throw new Error("readiness timeout") }),
      ])
      ready = true
      break
    } catch {
      await sleep(100)
    } finally {
      await probe.close()
    }
  }
  assert.ok(ready, "isolated kernel must become ready")
  client = new LocalIpcClient(endpoint, { localAuthEnvironment: kernelEnv, controlRequestRetryDeadlineMs: 1000, controlResponseStallMs: 1000 })
  const before = await client.send(cloudRelayStatusRequest())
  assert.equal(before.CloudRelayStatus.profile, null)
  // Legacy CLIs on the NEW kernel do not opt in. Keep their old terminal outcome.
  for (const request of [pollCloudRelayLoginRequest(apiUrl, "synthetic-device-code"), {
    PollCloudRelayLogin: { api_url: apiUrl, device_code: "synthetic-device-code", supports_access_denied: false },
  }]) {
    const legacy = await client.send(request)
    assert.equal(legacy.CloudRelayLoginPolled.result.status, "expired_token")
    assert.equal(legacy.CloudRelayLoginPolled.result.profile, null)
  }
  const legacyHttp = await fetch(`${apiUrl}/auth/device/poll`, {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ deviceCode: "synthetic-device-code" }),
  })
  assert.deepEqual(await legacyHttp.json(), { status: "expired_token" })
  const wire = await client.send(pollCloudRelayLoginRequest(apiUrl, "synthetic-device-code", true))
  assert.deepEqual(wire, { CloudRelayLoginPolled: { result: {
    status: "access_denied", interval_seconds: null, expires_at: null, profile: null,
  } } })
  assert.deepEqual(await pollCloudRelayLogin(client, apiUrl, "synthetic-device-code"), { status: "access_denied" })
  assert.deepEqual(await pollCloudDeviceLogin(apiUrl, "synthetic-device-code"), { status: "access_denied" })
  const notices = []
  let lifecyclePolls = 0
  await startHostedCloudLink({
    cloudRelayApiUrl: apiUrl, appendNotice: (message) => notices.push(message),
    flashFooter() {}, formatError: String,
    getRelayStatus: async () => (await client.send(relayStatusRequest())).RelayStatus.status,
    startCloudDeviceLogin: (url, input) => startCloudRelayLogin(client, url, input),
    pollCloudDeviceLogin: (url, code) => {
      lifecyclePolls += 1
      return pollCloudRelayLogin(client, url, code)
    },
    saveCloudRelayProfile: async () => assert.fail("denial must not save credentials"),
    configureRelay: async () => assert.fail("denial must not configure relay"),
  })
  assert.equal(lifecyclePolls, 1)
  assert.equal(notices.at(-1), "cloud login denied by the account owner")
  const after = await client.send(cloudRelayStatusRequest())
  assert.equal(after.CloudRelayStatus.profile, null)
  assert.equal((await client.send(relayStatusRequest())).RelayStatus.status.configured, false)
  // An upstream server that ignores capability negotiation must still not break
  // a legacy CLI attached to this kernel.
  forceUnadvertisedDenial = true
  const unadvertised = await client.send(pollCloudRelayLoginRequest(apiUrl, "synthetic-device-code"))
  assert.deepEqual(unadvertised.CloudRelayLoginPolled.result, {
    status: "expired_token", interval_seconds: null, expires_at: null, profile: null,
  })
  assert.equal(polls, 8)
  assert.deepEqual(advertised, [false, false, false, true, true, true, true, false])
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 436)
  // Upgraded kernels and direct CLIs must also work before Cloud is upgraded.
  rejectedPoll = undefined
  legacyServer = true
  legacyRequests.length = 0
  for (const poll of [
    () => pollCloudRelayLogin(client, apiUrl, "synthetic-device-code"),
    () => pollCloudDeviceLogin(apiUrl, "synthetic-device-code"),
  ]) {
    assert.equal((await poll()).status, "authorization_pending")
    const approved = await poll()
    assert.equal(approved.status, "approved")
    assert.equal(approved.profile.accountId, "fixture-account")
  }
  const approvedProfile = (await client.send(cloudRelayStatusRequest())).CloudRelayStatus.profile
  assert.equal(approvedProfile.account_id, "fixture-account")
  assert.equal((await client.send(relayStatusRequest())).RelayStatus.status.configured, false)
  assert.deepEqual(legacyRequests, [true, false, true, false, true, false, true, false].map((advertised) => ({
    deviceCode: "synthetic-device-code", ...(advertised ? { supportsAccessDenied: true } : {}),
  })))
  for (const [status, code, message, expectedRequests] of [
    [401, "invalid_request", "Request validation failed", 1],
    [403, "authorization_denied", "Forbidden", 1],
    [500, "invalid_request", "Request validation failed", 1],
    [400, "authorization_denied", "Request validation failed", 1],
    [400, "invalid_request", "Invalid device code", 1],
    [400, "invalid_request", "Request validation failed", 2],
  ]) {
    legacyRequests.length = 0
    rejectedPoll = { status, code, message }
    await assert.rejects(pollCloudRelayLogin(client, apiUrl, "synthetic-device-code"), (error) => {
      assert.ok(String(error).includes(`cloud relay request failed with ${status}: cloud_api_code=${code}:`))
      assert.ok(String(error).includes(message))
      return true
    })
    assert.equal(legacyRequests.length, expectedRequests, `${status} ${code}: ${message}`)
    assert.deepEqual((await client.send(cloudRelayStatusRequest())).CloudRelayStatus.profile, approvedProfile)
  }
  rejectedPoll = undefined
  console.log(JSON.stringify({ passed: true, protocolVersion: LOCAL_DAEMON_PROTOCOL_VERSION,
    legacyKernelCliStatus: "expired_token", legacyDirectCliStatus: "expired_token", unadvertisedDenialRetainsLegacyStatus: true,
    advertised, kernelStatus: wire.CloudRelayLoginPolled.result.status, directCliStatus: "access_denied",
    lifecyclePolls, notice: notices.at(-1), denialCredentialsSaved: false, relayConfigured: false,
    legacyServerKernelStatus: "approved", legacyServerDirectCliStatus: "approved", boundedValidationFallback: true,
    privateHome }, null, 2))
} finally {
  await client?.close()
  if (kernel.exitCode === null && kernel.signalCode === null) {
    kernel.kill("SIGTERM")
    await Promise.race([once(kernel, "exit"), sleep(5_000, undefined, { ref: false })])
    if (kernel.exitCode === null && kernel.signalCode === null) {
      kernel.kill("SIGKILL")
      await once(kernel, "exit")
    }
  }
  await new Promise((resolve) => server.close(resolve))
}
