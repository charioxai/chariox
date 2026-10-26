import { spawn } from "node:child_process"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { resolveBuiltBinary } from "./drill-runtime-helpers.mjs"
import { createCloudDrillClient, loadCloudRelayDrillModules } from "./live-cloud-relay-drill-helpers.mjs"
export { connectSessionScopedCloudClient, createCloudDrillClient, loadCloudRelayDrillModules, waitForCloudRelayTarget } from "./live-cloud-relay-drill-helpers.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..", "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const apiUrl = (process.env.CHARIOX_CLOUD_HOSTED_API_URL ?? "https://staging.chariox.com").replace(/\/$/, "")
const remoteCliHost = process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_HOST ?? "root@195.201.123.115"
const remoteCliKey = process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_KEY ?? path.join(os.homedir(), ".ssh/chariox_hetzner_staging")
const devAuthSecret = process.env.CHARIOX_CLOUD_DEV_AUTH_SECRET ?? ""

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export function log(name, details = null) {
  if (details == null) console.log(`[MP-08/MP-10/MP-11 hosted-cloud-relay-drill] ${name}`)
  else console.log(`[MP-08/MP-10/MP-11 hosted-cloud-relay-drill] ${name}`, JSON.stringify(details))
}

export function assert(condition, message) {
  if (!condition) throw new Error(message) // Responses can contain credentials.
}

export async function run(command, args, options = {}) {
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

export function spawnProcess(command, args, options) {
  const child = spawn(command, args, {
    ...options,
    stdio: ["ignore", "pipe", "pipe"],
  })
  const name = options.name ?? path.basename(command)
  // Service output is private runtime data, never evidence.
  child.stdout.resume()
  child.stderr.resume()
  child.on("exit", (code, signal) => {
    log(`${name}:exit`, { code, signal })
  })
  return child
}

export function shellQuote(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`
}

export function sshArgs(command, options = {}) {
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

export async function runSsh(command, options = {}) {
  return await run("ssh", sshArgs(command, options), {
    cwd: options.cwd ?? repoRoot,
    env: options.env ?? process.env,
  })
}

export async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null || child.signalCode != null) return
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("refusing to signal an invalid child PID")
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(5_000)])
  if (child.exitCode == null && child.signalCode == null) {
    if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("refusing to signal an invalid child PID")
    child.kill("SIGKILL")
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  }
}

export async function closeClient(client, label) {
  if (!client) return
  let timedOut = false
  let timeout = null
  await Promise.race([
    client.close().catch(() => {}),
    new Promise((resolve) => {
      timeout = setTimeout(() => {
        timedOut = true
        resolve()
      }, 2_000)
    }),
  ])
  if (timeout) clearTimeout(timeout)
  if (timedOut) {
    log("client-close-timeout", { label })
    client.destroy?.()
  }
}

export async function buildKernelIfNeeded() {
  const manifest = path.join(repoRoot, "apps/kernel/Cargo.toml")
  const binary = path.join(repoRoot, "apps/kernel/target/debug/chariox-kernel")
  const result = await run("cargo", ["build", "--manifest-path", manifest, "--bin", "chariox-kernel"])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return await resolveBuiltBinary(binary, manifest, "chariox-kernel")
}

export async function resolveExecutable(command) {
  if (command.includes(path.sep)) {
    const result = await run("test", ["-x", command])
    if (result.code === 0) return command
    throw new Error(`executable ${command} not found`)
  }
  const result = await run("sh", ["-lc", `command -v ${shellQuote(command)}`])
  if (result.code !== 0) throw new Error(`executable ${command} not found on PATH`)
  return result.stdout.trim()
}

export async function getFreePort() {
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

export async function makePorts() {
  return {
    kernelPort: await getFreePort(),
    mcpPort: await getFreePort(),
    homeOnlyMcpPort: await getFreePort(),
    opencodePort: await getFreePort(),
    codexPort: await getFreePort(),
  }
}

export async function makeWorkerPorts() {
  return {
    kernelPort: await getFreePort(),
    mcpPort: await getFreePort(),
    opencodePort: await getFreePort(),
    codexPort: await getFreePort(),
  }
}

export function unwrap(resp, key) {
  return resp?.[key] ?? resp
}

export async function postJson(url, body, headers = {}) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  })
  if (!response.ok) {
    throw new Error(`Cloud drill request failed (HTTP ${response.status})`)
  }
  return response.json().catch(() => null)
}

export async function cleanupHostedCloudIdentity({
  profile,
  cloudSessionToken = profile?.cloudSessionToken,
  clientIds = [],
  machineIds = [],
  kernelPresences = [],
  reason = "hosted Cloud drill cleanup",
  logout = true,
  baseUrl = apiUrl,
  post = postJson,
  logger = log,
}) {
  assert(profile?.accountId, "hosted Cloud cleanup requires an account id")
  const uniqueClientIds = [...new Set(clientIds.filter(Boolean))]
  const uniqueMachineIds = [...new Set(machineIds.filter(Boolean))]
  const presences = kernelPresences.filter((presence) => presence?.machineId && presence?.kernelId)
  const revokeClient = Boolean(profile.clientId && uniqueClientIds.includes(profile.clientId))
  const revokeMachine = Boolean(profile.machineId && uniqueMachineIds.includes(profile.machineId))
  const endSession = Boolean(logout || revokeClient || revokeMachine)
  if (endSession || presences.length || uniqueClientIds.length || uniqueMachineIds.length) {
    assert(
      typeof cloudSessionToken === "string" && cloudSessionToken.trim(),
      "hosted Cloud cleanup requires an authenticated Cloud session",
    )
  }
  const authorization = { authorization: `Bearer ${cloudSessionToken}` }
  const cleanupErrors = []

  for (const presence of presences) {
    await post(`${baseUrl}/kernels/presence`, {
      ...(cloudSessionToken ? { sessionToken: cloudSessionToken } : {}),
      accountId: profile.accountId,
      realmId: profile.realmId,
      machineId: presence.machineId,
      kernelId: presence.kernelId,
      status: "OFFLINE",
    }).catch((error) => cleanupErrors.push(error))
  }
  for (const clientId of uniqueClientIds.filter((id) => id !== profile.clientId)) {
    await post(`${baseUrl}/clients/revoke`, {
      accountId: profile.accountId,
      clientId,
      reason,
    }, authorization).catch((error) => cleanupErrors.push(error))
  }
  for (const machineId of uniqueMachineIds.filter((id) => id !== profile.machineId)) {
    await post(`${baseUrl}/machines/revoke`, {
      accountId: profile.accountId,
      machineId,
      reason,
    }, authorization).catch((error) => cleanupErrors.push(error))
  }
  // Retain authority to retry earlier cleanup failures. Own revocation consumes
  // this session even with logout:false, so combine both identities only at the end.
  const logoutAttempted = endSession && cleanupErrors.length === 0
  if (logoutAttempted) {
    await post(`${baseUrl}/auth/logout`, {
      sessionToken: cloudSessionToken,
      accountId: profile.accountId,
      ...(profile.clientId ? { clientId: profile.clientId } : {}),
      ...(profile.machineId ? { machineId: profile.machineId } : {}),
      revokeClient,
      revokeMachine,
    }).catch((error) => cleanupErrors.push(error))
  }
  logger("cloud-identity-cleanup", {
    accountSlug: profile.accountSlug,
    clients: uniqueClientIds,
    machines: uniqueMachineIds,
    kernels: presences.map((presence) => presence.kernelId),
    logout: logoutAttempted,
  })
  if (cleanupErrors.length > 0) {
    throw new AggregateError(cleanupErrors, "hosted Cloud identity cleanup failed")
  }
}

export async function createPairingToken({ accountId, userId, subjectKind, cloudSessionToken }) {
  assert(
    typeof cloudSessionToken === "string" && cloudSessionToken.trim(),
    "hosted Cloud pairing requires an authenticated Cloud session",
  )
  const response = await postJson(`${apiUrl}/pairing-tokens`, {
    accountId,
    createdByUserId: userId,
    subjectKind,
  }, { authorization: `Bearer ${cloudSessionToken}` })
  assert(response?.token, "cloud pairing token should be returned", response)
  return response.token
}

export async function pairCloudMachineDirect({ profile, machineId, alias }) {
  const token = await createPairingToken({
    accountId: profile.accountId,
    userId: profile.userId,
    subjectKind: "machine",
    cloudSessionToken: profile.cloudSessionToken,
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

export async function approveDevDeviceLogin({ role, userCode, accountSlug }) {
  if (!devAuthSecret) return false
  const slug = accountSlug ?? `hosted-${role}-${process.pid}-${Date.now()}`
  const email = `${slug}@chariox.local`
  log(`${role}-dev-approve-cloud-login`, { accountSlug: slug, email })
  await postJson(`${apiUrl}/auth/dev/device/approve`, {
    userCode,
    email,
    accountSlug: slug,
    displayName: `Hosted ${role} drill`,
    providerSubject: `dev|${slug}`,
  }, {
    "x-chariox-dev-auth-secret": devAuthSecret,
  })
  return true
}

export async function expectReject(promise, label, expectedText) {
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

export async function waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace, localAuthEnvironment) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl, { localAuthEnvironment })
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

export async function allowDevStubProvider(client, requests, label) {
  log("allow-dev-stub-provider", { label })
  await client.send(requests.setUserConfigValueRequest("providers.workspace_live_sync", "off"))
}

/** MP-08 / MP-10 / MP-11: each human authenticates through the ordinary CLIENT
 * flow with an isolated product identity and private credential store. */
export async function manualCloudDeviceLogin({ role, stateRoot, modules, terminal, accountSlug, expectedAccountId, baseUrl = apiUrl, approve = approveDevDeviceLogin }) {
  modules ??= await loadCloudRelayDrillModules()
  terminal ??= createCloudDrillClient(modules, stateRoot, `${role}-terminal`)
  try {
    const profile = await terminal.client.login(baseUrl, async verification => {
      log(`${role}-approve-cloud-login`, verification)
      await approve({ role, userCode: verification.userCode, accountSlug })
    }, expectedAccountId)
    return { ...terminal, profile }
  } catch (error) { terminal.client.stop(); throw error }
}

export async function cleanupHostedCloudTerminal(terminal, options = {}) {
  try {
    const credential = await terminal.client.store.session(terminal.identity.publicKeyThumbprint)
    await cleanupHostedCloudIdentity({
      ...options, profile: credential.profile, cloudSessionToken: credential.accessToken,
      clientIds: options.clientIds ?? [credential.clientId],
    })
    if (options.logout !== false) await terminal.client.store.clear()
  } finally { terminal.client.stop() }
}

export async function sendWithRetry(client, request, label, attempts = 5) {
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

export function isRetryableClientSendError(error) {
  if (error?.retryable === true) {
    return true
  }
  const message = error instanceof Error ? error.message : String(error)
  return /ETIMEDOUT|ECONNRESET|ECONNREFUSED|socket hang up|websocket closed|connection_closed/i.test(message)
}

export function installSendRetry(client, label) {
  const send = client.send.bind(client)
  client.send = (request) => sendWithRetry({ send }, request, label)
  return client
}

export async function waitForRemoteMachine(client, requests, machineRef) {
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

export async function waitForCompletion(eventLog, timeoutMs, baselineCount = 0) {
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

export async function waitForHistoryText(client, requests, sessionId, agentId, needle, timeoutMs, pollMs = 2_000, options = {}) {
  const providerOutputOnly = options.providerOutputOnly === true
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const outline = unwrap(
      await client.send(requests.getSessionHistoryOutlineRequest(sessionId, agentId ? [agentId] : null, 8)),
      "SessionHistoryOutline",
    )
    const chunks = []
    for (const agent of outline.agents ?? []) {
      for (const turn of agent.turns ?? []) {
        for (const entry of [turn.user_prompt, ...(turn.entries ?? []), turn.summary].filter(Boolean)) {
          if (!providerOutputOnly || historyEntryIsProviderOutput(entry)) {
            chunks.push(String(entry.entry?.text ?? entry.text ?? ""))
          }
        }
        for (const blob of turn.blobs ?? []) {
          const content = unwrap(
            await client.send(requests.getSessionHistoryBlobContentRequest(sessionId, agent.agent_id, blob.blob_id)),
            "SessionHistoryBlobContent",
          )
          for (const entry of content.entries ?? []) {
            if (!providerOutputOnly || historyEntryIsProviderOutput(entry)) {
              chunks.push(String(entry.entry?.text ?? entry.text ?? ""))
            }
          }
        }
      }
    }
    const text = chunks.join("")
    if (text.includes(needle)) return text
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for history text ${needle}`)
}

export function historyEntryIsProviderOutput(entry) {
  const kind = String(entry.entry?.kind ?? entry.kind ?? "")
  return kind === "provider_output" || kind === "assistant_message"
}

export async function waitForSession(client, requests, sessionId, timeoutMs = 20_000, pollMs = 500) {
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

/** MP-08 / MP-10 / MP-11: kernel enrollment and terminal sign-in have separate
 * authorities. Use product adapters; never extract human tokens from IPC. */
export function createHostedCommandDeps({ workspace, localClient, profileRef, notices, ownerAccountSlug, terminal, modules, baseUrl = apiUrl, approve = approveDevDeviceLogin }) {
  const appendNotice = message => {
    notices.push(message)
    log("command-notice", { firstLine: message.split("\n")[0] })
  }
  const openUrl = async url => {
    const userCode = new URL(url).searchParams.get("user_code")
    if (userCode) await approve({role: "owner", userCode, accountSlug: ownerAccountSlug})
    return false
  }
  return {
    workspace, worktree: workspace, clientId: `cli:${terminal.identity.publicKeyThumbprint}`,
    isAttached: () => false,
    sessionState: () => ({id: null, agents: [], workflows: []}),
    flashFooter: (message, tone) => {
      if (tone === "error") throw new Error(message)
      log("command-footer", {tone, message})
    },
    appendNotice,
    formatError: error => error instanceof Error ? error.message : String(error),
    cloudRelayApiUrl: baseUrl,
    getCloudRelayProfile: () => modules.relayApi.getKernelCloudRelayProfile(localClient),
    getCloudControlProfile: () => terminal.client.humanProfile(),
    getCloudCollaborationProfile: () => terminal.client.humanProfile(),
    saveCloudRelayProfile: async profile => {profileRef.current = profile},
    startCloudDeviceLogin: (url, input) => modules.relayApi.startCloudRelayLogin(localClient, url, input),
    pollCloudDeviceLogin: (url, deviceCode) => modules.relayApi.pollCloudRelayLogin(localClient, url, deviceCode),
    connectCloudRelay: () => modules.relayApi.connectKernelCloudRelay(localClient),
    getRelayStatus: () => modules.relayApi.getRelayStatus(localClient),
    openExternalUrl: openUrl,
    handleClientCloudCommand: modules.createCloudClientCommands({
      client: terminal.client, isKernelConnected: () => true, apiUrl: () => baseUrl,
      notice: appendNotice, saveProfile: async () => {}, refresh: async () => {}, openUrl,
    }),
    refreshWaitingRoomData: async () => {},
  }
}
