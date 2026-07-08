import { spawn } from "node:child_process"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..", "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const apiUrl = (process.env.ARROBA_CLOUD_HOSTED_API_URL ?? "https://arroba-cloud-staging.osc-fr1.scalingo.io").replace(/\/$/, "")
const pollTimeoutMs = Number(process.env.ARROBA_CLOUD_HOSTED_POLL_TIMEOUT_MS ?? 10 * 60 * 1000)
const remoteCliHost = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_HOST ?? "root@195.201.123.115"
const remoteCliKey = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_KEY ?? path.join(os.homedir(), ".ssh/arroba_hetzner_staging")
const devAuthSecret = process.env.ARROBA_CLOUD_DEV_AUTH_SECRET ?? ""

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export function log(name, details = null) {
  if (details == null) console.log(`[hosted-cloud-relay-drill] ${name}`)
  else console.log(`[hosted-cloud-relay-drill] ${name}`, JSON.stringify(details))
}

export function assert(condition, message, details = null) {
  if (!condition) {
    throw new Error(`${message}${details == null ? "" : `\n${JSON.stringify(details, null, 2)}`}`)
  }
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
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(5_000)])
  if (child.exitCode == null) {
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
  const binary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const result = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "arroba-kernel"])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
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

export function profileFromKernel(profile, expiresAt) {
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

export function tokenFromKernel(token, profile) {
  assert(token, "kernel cloud token response should include a token")
  return {
    relayUrl: token.relay_url,
    relayToken: token.relay_token,
    tokenExpiresAtMs: Date.parse(token.token_expires_at),
    profile: profile ? profileFromKernel(profile) : undefined,
  }
}

export function parseCloudClientTokenNotice(notices) {
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

export async function postJson(url, body, headers = {}) {
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

export async function createPairingToken({ accountId, userId, subjectKind }) {
  const response = await postJson(`${apiUrl}/pairing-tokens`, {
    accountId,
    createdByUserId: userId,
    subjectKind,
  })
  assert(response?.token, "cloud pairing token should be returned", response)
  return response.token
}

export async function pairCloudMachineDirect({ profile, machineId, alias }) {
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

export async function issueMachineRelayToken({ profile, machineId }) {
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

export async function approveDevDeviceLogin({ role, userCode, accountSlug }) {
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

export async function issueSessionScopedClientToken(apiUrl, {
  sessionToken,
  accountId,
  realmId,
  subject,
  userId,
  clientId,
  sessionId,
  targetDaemonAlias,
  allowUnpairedClientSubject,
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
    ...(allowUnpairedClientSubject ? { allowUnpairedClientSubject: true } : {}),
  })
  assert(runtime.token, "session-scoped relay token should be returned", runtime)
  return runtime.token
}

export async function waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace) {
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

export async function allowDevStubProvider(client, requests, label) {
  log("allow-dev-stub-provider", { label })
  await client.send(requests.setUserConfigValueRequest("providers.workspace_live_sync", "off"))
}

export async function manualCloudDeviceLogin({ role, clientId, clientAlias, localClient, requests }) {
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

export function appendCookies(jar, response) {
  const setCookie = response.headers.getSetCookie?.() ?? []
  for (const cookie of setCookie) {
    const pair = cookie.split(";", 1)[0]
    const index = pair.indexOf("=")
    if (index > 0) jar.set(pair.slice(0, index), pair.slice(index + 1))
  }
}

export function cookieHeader(jar) {
  return [...jar].map(([name, value]) => `${name}=${value}`).join("; ")
}

export async function devBrowserCloudLogin({ role }) {
  if (!devAuthSecret) {
    throw new Error("ARROBA_CLOUD_DEV_AUTH_SECRET is required for hosted browser dev login")
  }
  const slug = `hosted-${role}-${process.pid}-${Date.now()}`
  const email = `ma.gutierrez.estevez+${slug}@gmail.com`
  const jar = new Map()
  const csrfResponse = await fetch(`${apiUrl}/auth/csrf`)
  if (!csrfResponse.ok) {
    throw new Error(`GET /auth/csrf failed with ${csrfResponse.status}: ${await csrfResponse.text()}`)
  }
  appendCookies(jar, csrfResponse)
  const csrf = await csrfResponse.json()
  log(`${role}-dev-browser-login`, { accountSlug: slug, email })
  const loginResponse = await fetch(`${apiUrl}/auth/dev/browser-login`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-arroba-dev-auth-secret": devAuthSecret,
      cookie: cookieHeader(jar),
    },
    body: JSON.stringify({
      email,
      accountSlug: slug,
      displayName: `Hosted ${role} drill`,
      providerSubject: `dev|${slug}`,
    }),
  })
  if (!loginResponse.ok) {
    throw new Error(`POST /auth/dev/browser-login failed with ${loginResponse.status}: ${await loginResponse.text()}`)
  }
  appendCookies(jar, loginResponse)
  const cloudSessionResponse = await fetch(`${apiUrl}/auth/cloud-session`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "csrf-token": csrf.csrfToken,
      cookie: cookieHeader(jar),
    },
    body: JSON.stringify({ accountSlug: slug, accountName: slug }),
  })
  if (!cloudSessionResponse.ok) {
    throw new Error(`POST /auth/cloud-session failed with ${cloudSessionResponse.status}: ${await cloudSessionResponse.text()}`)
  }
  const result = await cloudSessionResponse.json()
  return {
    profile: {
      email: result.profile.email,
      accountId: result.profile.accountId,
      userId: result.profile.userId,
      accountSlug: result.profile.accountSlug,
      realmId: result.profile.realmId,
      relayUrl: result.profile.relayUrl,
      issuerId: result.profile.issuerId,
    },
    cloudSessionToken: result.cloudSessionToken,
  }
}

export async function waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, targetDaemonAlias) {
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

export async function handleRelayCommandWithRetry(handlers, command, label, attempts = 3) {
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

export function createHostedCommandDeps({
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
