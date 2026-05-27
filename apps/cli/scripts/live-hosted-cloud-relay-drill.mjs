#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, rm } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { runHostedSecondKernelAssertions, runHostedTokenRotationAssertions } from "./lib/hosted-cloud-kernel-scenarios.mjs"
import { runHostedMultiUserAssertions } from "./lib/hosted-cloud-multi-user-scenarios.mjs"
import { runHostedRemoteCliAssertions, runHostedRemoteCliPairingAssertions } from "./lib/hosted-cloud-remote-cli-scenarios.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const apiUrl = (process.env.ARROBA_CLOUD_HOSTED_API_URL ?? "https://arroba-cloud-staging.osc-fr1.scalingo.io").replace(/\/$/, "")
const pollTimeoutMs = Number(process.env.ARROBA_CLOUD_HOSTED_POLL_TIMEOUT_MS ?? 10 * 60 * 1000)
const runMultiUser = process.env.ARROBA_CLOUD_HOSTED_MULTI_USER === "1"
const runSecondKernel = process.env.ARROBA_CLOUD_HOSTED_SECOND_KERNEL === "1"
const runRemoteCli = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI === "1"
const runRemoteCliPairing = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_PAIRING === "1"
const runTokenRotation = process.env.ARROBA_CLOUD_HOSTED_TOKEN_ROTATION === "1"
const remoteCliPairingProvider = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_PROVIDER ?? "codex"
const remoteCliPairingModel = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_MODEL ?? "gpt-5.2-codex"
const remoteCliPairingEffort = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_EFFORT ?? "low"
const remoteCliHost = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_HOST ?? "root@195.201.123.115"
const remoteCliKey = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_KEY ?? path.join(os.homedir(), ".ssh/arroba_hetzner_staging")
const remoteCliRepo = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_REPO ?? "/opt/arroba-cli-drill"
const devAuthSecret = process.env.ARROBA_CLOUD_DEV_AUTH_SECRET ?? ""

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function log(name, details = null) {
  if (details == null) console.log(`[hosted-cloud-relay-drill] ${name}`)
  else console.log(`[hosted-cloud-relay-drill] ${name}`, JSON.stringify(details))
}

function assert(condition, message, details = null) {
  if (!condition) {
    throw new Error(`${message}${details == null ? "" : `\n${JSON.stringify(details, null, 2)}`}`)
  }
}

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.on("error", reject)
    child.on("close", (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

function spawnProcess(command, args, options) {
  const child = spawn(command, args, {
    ...options,
    stdio: ["ignore", "pipe", "pipe"],
  })
  const name = options.name ?? path.basename(command)
  if (options.logStdout !== false) {
    child.stdout.on("data", (chunk) => {
      for (const line of chunk.toString().trimEnd().split("\n").filter(Boolean)) {
        log(`${name}:stdout`, line)
      }
    })
  }
  if (options.logStderr !== false) {
    child.stderr.on("data", (chunk) => {
      for (const line of chunk.toString().trimEnd().split("\n").filter(Boolean)) {
        log(`${name}:stderr`, line)
      }
    })
  }
  child.on("exit", (code, signal) => {
    log(`${name}:exit`, { code, signal })
  })
  return child
}

function shellQuote(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`
}

function sshArgs(command, options = {}) {
  const args = [
    "-i",
    options.key ?? remoteCliKey,
    "-o",
    "IdentitiesOnly=yes",
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
  ]
  if (options.tty) args.push("-tt")
  args.push(options.host ?? remoteCliHost, command)
  return args
}

async function runSsh(command, options = {}) {
  return await run("ssh", sshArgs(command, options), {
    cwd: options.cwd ?? repoRoot,
    env: options.env ?? process.env,
  })
}

async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  }
}

async function closeClient(client, label) {
  if (!client) return
  let timedOut = false
  await Promise.race([
    client.close().catch(() => {}),
    sleep(2_000).then(() => {
      timedOut = true
      log("client-close-timeout", { label })
    }),
  ])
  if (timedOut) {
    client.controlWebsocket?.terminate?.()
    client.eventWebsocket?.terminate?.()
  }
}

async function buildKernelIfNeeded() {
  const binary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const result = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "arroba-kernel"])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

async function getFreePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer()
    server.on("error", reject)
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      server.close(() => {
        if (address && typeof address === "object") {
          resolve(address.port)
        } else {
          reject(new Error("failed to allocate a free port"))
        }
      })
    })
  })
}

async function makePorts() {
  return {
    kernelPort: await getFreePort(),
    mcpPort: await getFreePort(),
    opencodePort: await getFreePort(),
    codexPort: await getFreePort(),
  }
}

async function makeWorkerPorts() {
  return {
    kernelPort: await getFreePort(),
    mcpPort: await getFreePort(),
    opencodePort: await getFreePort(),
    codexPort: await getFreePort(),
  }
}

function unwrap(resp, key) {
  return resp?.[key] ?? resp
}

function profileFromKernel(profile, expiresAt) {
  assert(profile, "kernel cloud response should include a profile")
  return {
    apiUrl: profile.api_url,
    email: profile.email,
    accountId: profile.account_id,
    userId: profile.user_id,
    accountSlug: profile.account_slug,
    realmId: profile.realm_id,
    relayUrl: profile.relay_url,
    issuerId: profile.issuer_id,
    ...(profile.client_id ? { clientId: profile.client_id } : {}),
    ...(profile.client_alias ? { clientAlias: profile.client_alias } : {}),
    ...(profile.machine_id ? { machineId: profile.machine_id } : {}),
    ...(profile.machine_alias ? { machineAlias: profile.machine_alias } : {}),
    ...(profile.machine_credential ? { machineCredential: profile.machine_credential } : {}),
    ...(profile.cloud_session_token ? { cloudSessionToken: profile.cloud_session_token } : {}),
    ...(expiresAt ? { cloudSessionExpiresAtMs: Date.parse(expiresAt) } : {}),
  }
}

function tokenFromKernel(token, profile) {
  assert(token, "kernel cloud token response should include a token")
  return {
    relayUrl: token.relay_url,
    relayToken: token.relay_token,
    tokenExpiresAtMs: Date.parse(token.token_expires_at),
    profile: profile ? profileFromKernel(profile) : undefined,
  }
}

function parseCloudClientTokenNotice(notices) {
  const notice = [...notices].reverse().find((item) => (
    item.startsWith("cloud relay client token\n") || item.startsWith("cloud client token\n")
  ))
  assert(notice, "cloud relay client-token command should append a token notice", notices)
  const fields = Object.fromEntries(
    notice
      .split("\n")
      .slice(1)
      .map((line) => {
        const index = line.indexOf("=")
        return index === -1 ? [line, ""] : [line.slice(0, index), line.slice(index + 1)]
      }),
  )
  const relayUrl = fields.relay_url ?? fields.transport
  assert(relayUrl, "client token notice should include relay_url or transport", fields)
  const tokenMatch = fields.command?.match(/\s--relay-token\s+(\S+)/)
  assert(tokenMatch?.[1], "client token command should include --relay-token", fields.command)
  return {
    relayUrl,
    relayToken: tokenMatch[1],
  }
}

async function postJson(url, body, headers = {}) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  })
  if (!response.ok) {
    throw new Error(`POST ${url} failed with ${response.status}: ${await response.text()}`)
  }
  return response.json().catch(() => null)
}

async function createPairingToken({ accountId, userId, subjectKind }) {
  const response = await postJson(`${apiUrl}/pairing-tokens`, {
    accountId,
    createdByUserId: userId,
    subjectKind,
  })
  assert(response?.token, "cloud pairing token should be returned", response)
  return response.token
}

async function pairCloudMachineDirect({ profile, machineId, alias }) {
  const token = await createPairingToken({
    accountId: profile.accountId,
    userId: profile.userId,
    subjectKind: "machine",
  })
  const response = await postJson(`${apiUrl}/machines/pair`, {
    accountId: profile.accountId,
    token,
    machineId,
    userId: profile.userId,
    alias,
  })
  assert(response?.machineId === machineId, "cloud machine pair should return the paired machine id", response)
  return response
}

async function issueMachineRelayToken({ profile, machineId }) {
  const response = await postJson(`${apiUrl}/relay/token`, {
    sessionToken: profile.cloudSessionToken,
    accountId: profile.accountId,
    subject: machineId,
    subjectKind: "machine",
    realmId: profile.realmId,
    userId: profile.userId,
    machineId,
  })
  assert(response?.token, "machine relay token should be returned", response)
  return response.token
}

async function approveDevDeviceLogin({ role, userCode, accountSlug }) {
  if (!devAuthSecret) return false
  const slug = accountSlug ?? `hosted-${role}-${process.pid}-${Date.now()}`
  const email = `${slug}@arroba.local`
  log(`${role}-dev-approve-cloud-login`, { accountSlug: slug, email })
  await postJson(`${apiUrl}/auth/dev/device/approve`, {
    userCode,
    email,
    accountSlug: slug,
    displayName: `Hosted ${role} drill`,
    providerSubject: `dev|${slug}`,
  }, {
    "x-arroba-dev-auth-secret": devAuthSecret,
  })
  return true
}

async function expectReject(promise, label, expectedText) {
  try {
    await promise
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    if (expectedText && !message.includes(expectedText)) {
      throw new Error(`${label} rejected with unexpected error: ${message}`)
    }
    return message
  }
  throw new Error(`${label} unexpectedly succeeded`)
}

async function issueSessionScopedClientToken(apiUrl, {
  sessionToken,
  accountId,
  realmId,
  subject,
  userId,
  clientId,
  sessionId,
  targetDaemonAlias,
}) {
  const runtime = await postJson(`${apiUrl}/relay/token`, {
    sessionToken,
    accountId,
    subject,
    subjectKind: "client",
    realmId,
    userId,
    clientId,
    sessionId,
    allowedTargets: [targetDaemonAlias],
  })
  assert(runtime.token, "session-scoped relay token should be returned", runtime)
  return runtime.token
}

async function waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const created = unwrap(
        await probe.send(requests.createSessionRequest(workspace, workspace)),
        "SessionCreated",
      )
      await probe.send(requests.endSessionRequest(created.session.id)).catch(() => {})
      await probe.close()
      return
    } catch (error) {
      lastError = error
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`local daemon did not become ready: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

async function allowDevStubProvider(client, requests, label) {
  log("allow-dev-stub-provider", { label })
  await client.send(requests.setWorkspaceLiveSyncModeRequest("unrestricted"))
}

async function manualCloudDeviceLogin({ role, clientId, clientAlias, localClient, requests }) {
  log(`${role}-cloud-login-start`, { apiUrl })
  const login = unwrap(
    await localClient.send(requests.startCloudRelayLoginRequest(apiUrl, {
      clientId,
      clientAlias,
    })),
    "CloudRelayLoginStarted",
  ).login
  const expiresAtMs = Math.min(Date.parse(login.expires_at), Date.now() + pollTimeoutMs)
  log(`${role}-approve-cloud-login`, {
    verificationUrl: login.verification_url,
    userCode: login.user_code,
    expiresAt: login.expires_at,
  })
  await approveDevDeviceLogin({ role, userCode: login.user_code })
  while (Date.now() < expiresAtMs) {
    const result = unwrap(
      await localClient.send(requests.pollCloudRelayLoginRequest(apiUrl, login.device_code)),
      "CloudRelayLoginPolled",
    ).result
    log(`${role}-cloud-login-poll-result`, { status: result.status })
    if (result.status === "approved") {
      assert(result.profile?.cloud_session_token, `${role} cloud login should return a cloud session token`, result)
      return {
        profile: profileFromKernel(result.profile, result.expires_at),
        cloudSessionToken: result.profile.cloud_session_token,
      }
    }
    if (result.status === "expired_token") {
      throw new Error(`${role} cloud login expired`)
    }
    await sleep(Math.max(result.interval_seconds ?? 2, 1) * 1000)
  }
  throw new Error(`${role} cloud login timed out`)
}

async function waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, targetDaemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 30; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await Promise.race([
        client.send(requests.listSessionsRequest()),
        sleep(2_000).then(() => { throw new Error("probe timeout") }),
      ])
      await client.close()
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      if (attempt === 0 || attempt % 10 === 9) {
        log("relay-target-wait-retry", {
          targetDaemonAlias,
          attempt: attempt + 1,
          error: error instanceof Error ? error.message : String(error),
        })
      }
      await sleep(250)
    }
  }
  throw new Error(`relay target did not become reachable: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

async function sendWithRetry(client, request, label, attempts = 5) {
  let lastError = null
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      return await client.send(request)
    } catch (error) {
      lastError = error
      if (!isRetryableClientSendError(error) || attempt === attempts - 1) {
        break
      }
      log("client-send-retry", {
        label,
        attempt: attempt + 1,
        error: error instanceof Error ? error.message : String(error),
      })
      await sleep(1_000 * (attempt + 1))
    }
  }
  throw lastError
}

function isRetryableClientSendError(error) {
  if (error?.retryable === true) {
    return true
  }
  const message = error instanceof Error ? error.message : String(error)
  return /ETIMEDOUT|ECONNRESET|ECONNREFUSED|socket hang up|websocket closed|connection_closed/i.test(message)
}

function installSendRetry(client, label) {
  const send = client.send.bind(client)
  client.send = (request) => sendWithRetry({ send }, request, label)
  return client
}

async function handleRelayCommandWithRetry(handlers, command, label, attempts = 3) {
  let lastError = null
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      return await handlers.handleRelayCommand(command)
    } catch (error) {
      lastError = error
      if (attempt === attempts - 1) {
        break
      }
      log("relay-command-retry", {
        label,
        attempt: attempt + 1,
        error: error instanceof Error ? error.message : String(error),
      })
      await sleep(1_000 * (attempt + 1))
    }
  }
  throw lastError
}

async function waitForRemoteMachine(client, requests, machineRef) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      const listed = unwrap(
        await Promise.race([
          client.send(requests.listRemoteMachineKernelsRequest(machineRef)),
          sleep(15_000).then(() => { throw new Error("remote machine kernel list timeout") }),
        ]),
        "RemoteMachineKernelsListed",
      )
      if ((listed.kernels ?? []).some((kernel) => kernel.accepting_remote_leases)) {
        return listed
      }
    } catch (error) {
      lastError = error
      if (attempt === 0 || attempt % 10 === 9) {
        log("remote-machine-wait-retry", {
          machineRef,
          attempt: attempt + 1,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not become ready: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

async function waitForCompletion(eventLog, timeoutMs, baselineCount = 0) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const completions = eventLog.filter((event) => event.event === "assistant_message_completed")
    if (completions.length > baselineCount) {
      return completions[completions.length - 1]
    }
    await sleep(100)
  }
  throw new Error("timed out waiting for assistant completion")
}

async function waitForHistoryText(client, requests, sessionId, agentId, needle, timeoutMs, pollMs = 2_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const history = unwrap(
      await client.send(requests.getSessionHistoryRequest(sessionId, 60, 120_000, null, agentId ?? null)),
      "SessionHistory",
    )
    const text = (history.entries ?? [])
      .map((entry) => String(entry.entry?.text ?? entry.text ?? ""))
      .join("")
    if (text.includes(needle)) return text
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for history text ${needle}`)
}

async function waitForSession(client, requests, sessionId, timeoutMs = 20_000, pollMs = 500) {
  const deadline = Date.now() + timeoutMs
  let lastListed = null
  while (Date.now() < deadline) {
    const listed = unwrap(
      await client.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    lastListed = listed
    const session = (listed.sessions ?? []).find((candidate) => candidate.id === sessionId)
    if (session) return session
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for session ${sessionId}\n${JSON.stringify(lastListed, null, 2)}`)
}

function createHostedCommandDeps({
  workspace,
  clientId,
  localClient,
  requests,
  profileRef,
  notices,
  ownerAccountSlug,
}) {
  return {
    workspace,
    worktree: workspace,
    clientId,
    isAttached: () => false,
    sessionState: () => ({ id: null, agents: [], workflows: [] }),
    attachmentState: () => null,
    providerRunState: () => null,
    currentModelId: () => "gpt-5.2",
    currentVariantId: () => "low",
    currentProviderId: () => "codex",
    focusedAgentId: () => null,
    multiAgentResponseLayout: () => "individual",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message, tone) => log("command-footer", { tone, message }),
    appendNotice: (message) => {
      notices.push(message)
      log("command-notice", { firstLine: message.split("\n")[0] })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    cloudRelayApiUrl: apiUrl,
    getCloudRelayProfile: () => profileRef.current,
    saveCloudRelayProfile: async (profile) => {
      profileRef.current = profile
    },
    bootstrapCloudRelay: async () => {
      throw new Error("hosted drill uses device login, not bootstrap")
    },
    pairCloudRelayClient: async (_profile, nextClientId, alias) => {
      const paired = unwrap(
        await localClient.send(requests.pairCloudRelayClientRequest(nextClientId, alias)),
        "CloudRelayClientPaired",
      )
      return profileFromKernel(paired.profile)
    },
    pairCloudRelayMachine: async (_profile, machineId, alias) => {
      const paired = unwrap(
        await localClient.send(requests.pairCloudRelayMachineRequest(machineId, alias)),
        "CloudRelayMachinePaired",
      )
      return profileFromKernel(paired.profile)
    },
    getRelayStatus: async () => unwrap(
      await localClient.send(requests.relayStatusRequest()),
      "RelayStatus",
    ).status,
    configureRelay: async (relayUrl, relayToken) => unwrap(
      await localClient.send(requests.configureRelayRequest(relayUrl, relayToken)),
      "RelayConfigured",
    ).status,
    startCloudDeviceLogin: async (nextApiUrl, input) => {
      log("kernel-cloud-login-start", { apiUrl: nextApiUrl })
      const login = unwrap(
        await localClient.send(requests.startCloudRelayLoginRequest(nextApiUrl, input)),
        "CloudRelayLoginStarted",
      ).login
      log("approve-cloud-login", {
        verificationUrl: login.verification_url,
        userCode: login.user_code,
        expiresAt: login.expires_at,
      })
      await approveDevDeviceLogin({
        role: "owner",
        userCode: login.user_code,
        accountSlug: ownerAccountSlug,
      })
      return {
        apiUrl: login.api_url,
        deviceCode: login.device_code,
        userCode: login.user_code,
        verificationUrl: login.verification_url,
        expiresAtMs: Math.min(Date.parse(login.expires_at), Date.now() + pollTimeoutMs),
        intervalSeconds: login.interval_seconds,
      }
    },
    pollCloudDeviceLogin: async (nextApiUrl, deviceCode) => {
      const result = unwrap(
        await localClient.send(requests.pollCloudRelayLoginRequest(nextApiUrl, deviceCode)),
        "CloudRelayLoginPolled",
      ).result
      log("kernel-cloud-login-poll-result", { status: result.status })
      if (result.status === "authorization_pending") {
        return {
          status: "authorization_pending",
          intervalSeconds: result.interval_seconds ?? 2,
          expiresAtMs: result.expires_at ? Date.parse(result.expires_at) : Date.now() + 30_000,
        }
      }
      if (result.status === "expired_token") {
        return { status: "expired_token" }
      }
      return {
        status: "approved",
        profile: profileFromKernel(result.profile, result.expires_at),
      }
    },
    issueCloudKernelRelayToken: async () => {
      const connected = unwrap(
        await localClient.send(requests.connectCloudRelayRequest()),
        "CloudRelayConnected",
      )
      return tokenFromKernel(connected.token, connected.profile)
    },
    issueCloudMachineRelayToken: async () => {
      const connected = unwrap(
        await localClient.send(requests.connectCloudRelayRequest()),
        "CloudRelayConnected",
      )
      return tokenFromKernel(connected.token, connected.profile)
    },
    issueCloudClientRelayToken: async (_profile, targetDaemonAlias, options = {}) => {
      const issued = unwrap(
        await localClient.send(requests.issueCloudRelayClientTokenRequest(targetDaemonAlias, clientId, options.sessionId)),
        "CloudRelayClientTokenIssued",
      )
      return tokenFromKernel(issued.token, issued.profile)
    },
    refreshWaitingRoomData: async () => {},
    openExternalUrl: async () => false,
  }
}

async function main() {
  const ports = await makePorts()
  const runId = `hosted-cloud-relay-${process.pid}-${Date.now()}`
  const rootDir = path.join(os.tmpdir(), runId)
  const workspace = path.join(rootDir, "workspace")
  const homeDir = path.join(rootDir, "home")
  const arrobaHome = path.join(homeDir, ".arroba")
  const xdgConfigHome = path.join(homeDir, ".config")
  const xdgStateHome = path.join(homeDir, ".local", "state")
  const xdgRuntimeDir = path.join(homeDir, "run")
  const daemonId = `hosted-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `hosted-home-${process.pid}`
  const clientId = `hosted-cli-${process.pid}-${Date.now()}`
  const ownerAccountSlug = `hosted-owner-${process.pid}-${Date.now()}`

  await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(workspace, { recursive: true })
  await mkdir(arrobaHome, { recursive: true })
  await mkdir(xdgConfigHome, { recursive: true })
  await mkdir(xdgStateHome, { recursive: true })
  await mkdir(xdgRuntimeDir, { recursive: true })

  let daemon = null
  let localClient = null
  let remoteClient = null
  let passed = false

  try {
    log("build-cli")
    const cliBuild = await run("pnpm", ["run", "build"], { cwd: cliRoot, env: process.env })
    if (cliBuild.code !== 0) {
      throw new Error(`arroba cli build failed\n${cliBuild.stdout}\n${cliBuild.stderr}`)
    }

    log("build-kernel")
    const kernelPath = await buildKernelIfNeeded()

    const [{ LocalIpcClient }, requests, commandActions] = await Promise.all([
      import("../../../packages/kernel-client/dist/ipc.js"),
      import("../../../packages/kernel-client/dist/ipc-requests.js"),
      import("../dist/command-actions.js"),
    ])

    const daemonEnv = {
      ...process.env,
      HOME: os.homedir(),
      XDG_CONFIG_HOME: xdgConfigHome,
      XDG_STATE_HOME: xdgStateHome,
      XDG_RUNTIME_DIR: xdgRuntimeDir,
      ARROBA_HOME: arrobaHome,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_DAEMON_ID: daemonId,
      ARROBA_DAEMON_ALIAS: daemonAlias,
      ARROBA_MACHINE_ID: daemonId,
      ARROBA_MACHINE_ALIAS: daemonAlias,
      ARROBA_DAEMON_SOCKET: path.join(rootDir, "daemon.sock"),
      ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, "session-history"),
    }

    log("start-kernel")
    daemon = spawnProcess(kernelPath, [], { cwd: repoRoot, env: daemonEnv, name: "kernel" })

    const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}/kernel`
    await waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace)
    localClient = new LocalIpcClient(kernelUrl)

    const profileRef = { current: null }
    const notices = []
    const handlers = commandActions.createCommandActionHandlers(createHostedCommandDeps({
      workspace,
      clientId,
      localClient,
      requests,
      profileRef,
      notices,
      ownerAccountSlug,
    }))

    log("command-cloud-login", { apiUrl })
    await handleRelayCommandWithRetry(handlers, {
      kind: "relay",
      raw: "/relay cloud login",
      args: ["cloud", "login"],
    }, "cloud-login")
    assert(profileRef.current?.cloudSessionToken, "hosted cloud login should save an authenticated profile", profileRef.current)

    log("command-cloud-pair")
    await handleRelayCommandWithRetry(handlers, {
      kind: "relay",
      raw: "/relay cloud pair hosted-drill-cli",
      args: ["cloud", "pair", "hosted-drill-cli"],
    }, "cloud-pair")
    assert(profileRef.current?.clientId === clientId, "hosted cloud pair should save client id", profileRef.current)

    log("command-cloud-pair-machine")
    await handleRelayCommandWithRetry(handlers, {
      kind: "relay",
      raw: `/relay cloud pair-machine ${daemonId} hosted-drill-machine`,
      args: ["cloud", "pair-machine", daemonId, "hosted-drill-machine"],
    }, "cloud-pair-machine")
    assert(profileRef.current?.machineId === daemonId, "hosted cloud pair-machine should save machine id", profileRef.current)

    log("command-cloud-connect")
    await handleRelayCommandWithRetry(handlers, {
      kind: "relay",
      raw: "/relay cloud connect",
      args: ["cloud", "connect"],
    }, "cloud-connect")

    log("command-cloud-client-token")
    await handleRelayCommandWithRetry(handlers, {
      kind: "relay",
      raw: `/relay cloud client-token ${daemonAlias}`,
      args: ["cloud", "client-token", daemonAlias],
    }, "cloud-client-token")
    const clientRelay = parseCloudClientTokenNotice(notices)

    log("relay-target-probe", { relayUrl: clientRelay.relayUrl, daemonAlias })
    await waitForRelayTarget(
      LocalIpcClient,
      requests,
      clientRelay.relayUrl,
      clientRelay.relayToken,
      daemonAlias,
    )

    remoteClient = installSendRetry(new LocalIpcClient(clientRelay.relayUrl, {
      relayAuthToken: clientRelay.relayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    }), "owner-relay")

    log("remote-session-create")
    const created = unwrap(
      await sendWithRetry(remoteClient, requests.createSessionRequest(workspace, workspace), "remote-session-create"),
      "SessionCreated",
    )
    assert(created.session?.id, "remote cloud session creation should return a session", created)

    const listed = unwrap(
      await sendWithRetry(remoteClient, requests.listSessionsRequest(), "remote-session-list"),
      "SessionsListed",
    )
    assert(
      Array.isArray(listed.sessions) && listed.sessions.some((session) => session.id === created.session.id),
      "remote cloud client should list the created session",
      listed,
    )

    if (runRemoteCli) {
      await runHostedRemoteCliAssertions({
        requests,
        homeClient: localClient,
        verificationClient: remoteClient,
        relayUrl: clientRelay.relayUrl,
        relayToken: clientRelay.relayToken,
        targetDaemonAlias: daemonAlias,
        repoRoot,
        remoteCliRepo,
        remoteCliHost,
        log,
        assert,
        unwrap,
        shellQuote,
        sshArgs,
        runSsh,
        spawnProcess,
        terminateChild,
        allowDevStubProvider,
      })
    }

    if (runRemoteCliPairing) {
      await runHostedRemoteCliPairingAssertions({
        requests,
        homeClient: localClient,
        verificationClient: remoteClient,
        workspace,
        kernelUrl,
        cliRoot,
        repoRoot,
        remoteCliRepo,
        remoteCliHost,
        remoteCliPairingProvider,
        remoteCliPairingModel,
        remoteCliPairingEffort,
        pollTimeoutMs,
        log,
        assert,
        unwrap,
        shellQuote,
        sshArgs,
        runSsh,
        spawnProcess,
        terminateChild,
        waitForSession,
        waitForHistoryText,
      })
    }

    if (runTokenRotation) {
      await runHostedTokenRotationAssertions({
        requests,
        homeClient: localClient,
        verificationClient: remoteClient,
        sessionId: created.session.id,
        log,
        assert,
        unwrap,
      })
    }

    if (runMultiUser) {
      await runHostedMultiUserAssertions({
        LocalIpcClient,
        requests,
        localClient,
        ownerRemoteClient: remoteClient,
        ownerProfile: profileRef.current,
        ownerClientId: clientId,
        workspace,
        daemonAlias,
        session: created.session,
        apiUrl,
        log,
        assert,
        unwrap,
        postJson,
        issueSessionScopedClientToken,
        manualCloudDeviceLogin,
        installSendRetry,
        expectReject,
      })
    } else {
      log("multi-user-skipped", {
        reason: devAuthSecret
          ? "set ARROBA_CLOUD_HOSTED_MULTI_USER=1"
          : "set ARROBA_CLOUD_HOSTED_MULTI_USER=1 and approve owner, peer, and third browser logins, or set ARROBA_CLOUD_DEV_AUTH_SECRET",
      })
    }

    if (runSecondKernel) {
      await runHostedSecondKernelAssertions({
        LocalIpcClient,
        requests,
        kernelPath,
        rootDir,
        workspace,
        homeClient: localClient,
        ownerProfile: profileRef.current,
        ownerClientId: clientId,
        apiUrl,
        repoRoot,
        pollTimeoutMs,
        log,
        assert,
        unwrap,
        makeWorkerPorts,
        pairCloudMachineDirect,
        issueMachineRelayToken,
        issueSessionScopedClientToken,
        waitForLocalDaemon,
        allowDevStubProvider,
        waitForRelayTarget,
        waitForRemoteMachine,
        waitForCompletion,
        closeClient,
        terminateChild,
        spawnProcess,
      })
    }

    log("pass", {
      apiUrl,
      relayUrl: clientRelay.relayUrl,
      accountSlug: profileRef.current.accountSlug,
      sessionId: created.session.id,
      multiUser: runMultiUser,
      remoteCli: runRemoteCli,
      remoteCliPairing: runRemoteCliPairing,
      secondKernel: runSecondKernel,
      tokenRotation: runTokenRotation,
    })
    passed = true
  } finally {
    await closeClient(remoteClient, "remote")
    await closeClient(localClient, "local")
    await terminateChild(daemon)
    if (passed) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      log("preserved-failed-run", { rootDir })
    }
  }
}

main().then(() => {
  process.exit(0)
}).catch((error) => {
  console.error(error)
  process.exitCode = 1
})
