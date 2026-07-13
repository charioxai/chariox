import { mkdir, readFile, realpath, writeFile } from "node:fs/promises"
import http from "node:http"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"
import { runNodeDrillChild } from "./drill-child-process.mjs"
import { withDevStubProviderInventory } from "./drill-runtime-helpers.mjs"
import {
  callRuntimeMcp,
  expectRuntimeMcpReject,
  runCommand,
  waitForRuntimeTool,
} from "./hosted-cloud-runtime-helpers.mjs"

export const HOSTED_HOME_PROXY_MODEL = "native-tui-idle"

export function withHostedKernelIsolation(env, {
  homeDir,
  arrobaHome,
  xdgConfigHome,
  xdgStateHome,
  xdgRuntimeDir,
}) {
  return {
    ...env,
    HOME: homeDir,
    ARROBA_HOME: arrobaHome,
    XDG_CONFIG_HOME: xdgConfigHome,
    XDG_STATE_HOME: xdgStateHome,
    XDG_RUNTIME_DIR: xdgRuntimeDir,
  }
}

async function initHostedLiveSyncWorktree(worktree, label) {
  await mkdir(path.join(worktree, "outputs"), { recursive: true })
  await mkdir(path.join(worktree, "ignored"), { recursive: true })
  await writeFile(path.join(worktree, "seed.txt"), `${label}-seed\n`, "utf8")
  await writeFile(path.join(worktree, ".arrobaignore"), "ignored/\n*.secret\n", "utf8")
  await runCommand("git", ["init"], worktree)
  await runCommand("git", ["config", "user.email", "hosted-workspace-live-sync@example.com"], worktree)
  await runCommand("git", ["config", "user.name", "Hosted Workspace Live Sync Drill"], worktree)
  await runCommand("git", ["add", "."], worktree)
  await runCommand("git", ["commit", "-m", "seed hosted workspace live sync fixture"], worktree)
}

async function waitForFileContent(filePath, expected, timeoutMs, pollMs = 250) {
  const deadline = Date.now() + timeoutMs
  let lastContent = null
  while (Date.now() < deadline) {
    try {
      lastContent = await readFile(filePath, "utf8")
      if (lastContent === expected) return
    } catch {
      lastContent = null
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${filePath} content ${JSON.stringify(expected)}; last=${JSON.stringify(lastContent)}`)
}

export async function startHostedRemoteProviderRun({
  client,
  requests,
  sessionId,
  attachmentId,
  agentId,
  prompt,
  timeoutMs = 90_000,
  pollMs = 250,
}) {
  await client.send(requests.submitPromptRequest(
    sessionId,
    attachmentId,
    agentId,
    prompt,
    [],
  ))

  const deadline = Date.now() + timeoutMs
  let lastAgent = null
  let lastRun = null
  while (Date.now() < deadline) {
    const listed = await client.send(requests.listAgentsRequest(sessionId))
    const agents = listed?.AgentsListed?.agents ?? listed?.agents ?? []
    const agent = agents.find((candidate) => candidate.id === agentId) ?? null
    lastAgent = agent
    const remote = agent?.remote_execution
    if (remote?.leased_agent_id && remote?.active_worker_provider_run_id) {
      const projectedRunId = `leased:${remote.leased_agent_id}:${remote.active_worker_provider_run_id}`
      const response = await client.send(requests.getProviderRunRequest(projectedRunId)).catch(() => null)
      const run = response?.ProviderRun?.provider_run ?? response?.provider_run ?? null
      lastRun = run
      if (run?.runtime_mcp_server_url && run?.runtime_mcp_auth_token) return run
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for hosted remote provider run for ${agentId}: agent=${JSON.stringify(lastAgent)} run=${JSON.stringify(lastRun)}`)
}

async function prepareHostedWorkspaceLiveSyncFixture({
  client,
  requests,
  session,
  workspace,
  workerWorkspace,
  rootDir,
  log,
  unwrap,
}) {
  const targetWorkspace = path.join(rootDir, "hosted-live-sync-target")
  await initHostedLiveSyncWorktree(workspace, "home")
  await initHostedLiveSyncWorktree(workerWorkspace, "worker")
  await initHostedLiveSyncWorktree(targetWorkspace, "target")
  const canonicalWorkspace = await realpath(workspace)
  const canonicalWorkerWorkspace = await realpath(workerWorkspace)
  const canonicalTargetWorkspace = await realpath(targetWorkspace)
  await client.send(requests.setWorkspaceLiveSyncModeRequest(session.id, "managed"))
  const linkName = `hosted-managed-live-sync-${Date.now()}`
  unwrap(await client.send(requests.createWorkspaceLinkRequest(session.id, linkName)), "WorkspaceLinkCreated")
  await client.send(requests.attachWorkspaceLinkRequest(session.id, linkName, canonicalWorkspace))
  await client.send(requests.attachWorkspaceLinkRequest(session.id, linkName, canonicalWorkerWorkspace))
  await client.send(requests.attachWorkspaceLinkRequest(session.id, linkName, canonicalTargetWorkspace))
  log("second-kernel-workspace-live-sync-prepared", {
    sessionId: session.id,
    mode: "managed",
    linkName,
    homeWorkspace: canonicalWorkspace,
    sourceWorkspace: canonicalWorkerWorkspace,
    targetWorkspace: canonicalTargetWorkspace,
  })
  return { linkName, targetWorkspace: canonicalTargetWorkspace }
}

async function assertHostedWorkspaceLiveSyncProxy({
  client,
  requests,
  session,
  launch,
  workerWorkspace,
  targetWorkspace,
  pollTimeoutMs,
  label = "single",
  log,
  unwrap,
}) {
  if (!launch.runtime_mcp_server_url || !launch.runtime_mcp_auth_token) {
    throw new Error(`launched run lacks runtime MCP binding for workspace live sync: ${JSON.stringify(launch)}`)
  }
  log("second-kernel-workspace-live-sync-tool-wait", { tool: "arroba.write_artifact" })
  await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "arroba.write_artifact", true)
  const relativePath = `outputs/hosted-managed-live-sync-${label}.txt`
  const expected = `HOSTED_MANAGED_WORKSPACE_LIVE_SYNC_${label.toUpperCase()}_OK\n`
  log("second-kernel-workspace-live-sync-write-start", { label, relativePath })
  const write = await callRuntimeMcp(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "tools/call", {
    name: "arroba.write_artifact",
    arguments: { path: relativePath, content_text: expected },
  })
  if (write.isError) throw new Error(`hosted workspace live sync write returned error: ${JSON.stringify(write)}`)
  log("second-kernel-workspace-live-sync-write-returned", { label, relativePath })
  await waitForFileContent(path.join(workerWorkspace, relativePath), expected, pollTimeoutMs)
  await waitForFileContent(path.join(targetWorkspace, relativePath), expected, pollTimeoutMs)

  const ignoredPath = `ignored/hosted-managed-live-sync-${label}.txt`
  log("second-kernel-workspace-live-sync-ignore-check", { label, path: ignoredPath })
  const ignored = await expectRuntimeMcpReject(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "tools/call", {
    name: "arroba.write_artifact",
    arguments: { path: ignoredPath, content_text: "SHOULD_NOT_WRITE\n" },
  })
  if (!JSON.stringify(ignored).includes("excluded from workspace live sync")) {
    throw new Error(`ignored hosted workspace live sync write rejected with unexpected result: ${JSON.stringify(ignored)}`)
  }
  if (ignored.isError === false && ignored.structuredContent?.applied !== false) {
    throw new Error(`ignored hosted workspace live sync write unexpectedly succeeded: ${JSON.stringify(ignored)}`)
  }

  const status = unwrap(
    await client.send(requests.getWorkspaceLiveSyncStatusRequest(session.id)),
    "WorkspaceLiveSyncStatus",
  ).status
  if (!(status.targets ?? []).some((target) => target.repo_root === targetWorkspace)) {
    throw new Error(`hosted workspace live sync status did not include target ${targetWorkspace}: ${JSON.stringify(status)}`)
  }
  log("second-kernel-workspace-live-sync-pass", {
    sessionId: session.id,
    label,
    sourceWorkspace: workerWorkspace,
    targetWorkspace,
    mode: "managed",
    relativePath,
  })
}

async function runHostedTrackedWorkspaceLiveSyncProviderDrill({
  rootDir,
  repoRoot,
  kernelUrl,
  workerDaemonId,
  homeHistoryDir,
  workerHistoryDir,
  provider,
  model,
  timeoutMs,
  pollMs,
  log,
}) {
  const drillRoot = path.join(rootDir, "hosted-tracked-workspace-live-sync")
  log("second-kernel-tracked-workspace-live-sync-start", {
    provider,
    model,
    workerDaemonId,
    drillRoot,
  })
  const stdout = await runNodeDrillChild([
    path.join("apps", "cli", "scripts", "live-workspace-live-sync-drill.mjs"),
    "--kernel", kernelUrl,
    "--no-spawn-daemon",
    "--machine-ref", workerDaemonId,
    "--history-dir", homeHistoryDir,
    "--provider-history-dir", workerHistoryDir,
    "--provider", provider,
    "--provider-model", `${provider}=${model}`,
    "--timeout-ms", String(timeoutMs),
    "--poll-ms", String(pollMs),
    "--mode", "tracked",
    "--tracked-target-count", "1",
    "--tracked-bidirectional",
    "--root-dir", drillRoot,
  ], repoRoot, { label: "hosted tracked workspace live sync drill" })
  const trimmed = stdout.trim()
  const lastJsonIndex = trimmed.lastIndexOf("\n{")
  const jsonText = lastJsonIndex >= 0 ? trimmed.slice(lastJsonIndex + 1) : trimmed
  const result = JSON.parse(jsonText)
  log("second-kernel-tracked-workspace-live-sync-pass", {
    provider,
    durationMs: result.durationMs ?? result.workspaceLiveSync?.durationMs ?? null,
    mode: result.mode ?? null,
  })
  return result
}

async function createHostedHomeExtensionFixtures({ rootDir, homeCapabilityRoot, homeOnlyMcpPort }) {
  const homeMarker = path.join(rootDir, "hosted-home-script-marker.txt")
  const homeMcpMarker = path.join(rootDir, "hosted-home-mcp-marker.txt")
  const homeConnectorMarker = path.join(rootDir, "hosted-home-connector-marker.txt")
  const scriptPath = path.join(rootDir, "hosted_home_only_lookup.py")
  await writeFile(scriptPath, `
MARKER = ${JSON.stringify(homeMarker)}

def run(query: str) -> dict[str, object]:
    """Return a deterministic hosted home-only lookup result."""
    with open(MARKER, "w", encoding="utf-8") as handle:
        handle.write("HOSTED_HOME_SCRIPT_EXECUTED:" + query)
    return {"query": query, "origin": "hosted-home"}

def test_run() -> None:
    result = run("self-test")
    assert result["origin"] == "hosted-home"
`, "utf8")

  const homeMcpDir = path.join(homeCapabilityRoot, "user", "mcps")
  await mkdir(homeMcpDir, { recursive: true })
  await writeFile(path.join(homeMcpDir, "hosted_home_echo_mcp.json"), `${JSON.stringify({
    name: "hosted_home_echo_mcp",
    transport: {
      type: "streamable_http",
      url: `http://127.0.0.1:${homeOnlyMcpPort}/mcp`,
    },
    enabled: true,
    required: false,
    tool_timeout_sec: 10,
  }, null, 2)}\n`, "utf8")

  const homeOnlyMcp = http.createServer(async (req, res) => {
    let body = ""
    req.setEncoding("utf8")
    for await (const chunk of req) body += chunk
    const rpc = body ? JSON.parse(body) : {}
    res.setHeader("content-type", "application/json")
    if (rpc.method === "tools/list") {
      return res.end(JSON.stringify({
        jsonrpc: "2.0",
        id: rpc.id ?? null,
        result: {
          tools: [{
            name: "hosted_home_echo",
            description: "Hosted home-only MCP echo tool.",
            inputSchema: {
              type: "object",
              required: ["text"],
              properties: { text: { type: "string" } },
              additionalProperties: false,
            },
          }],
        },
      }))
    }
    if (rpc.method === "tools/call" && rpc.params?.name === "hosted_home_echo") {
      const text = String(rpc.params?.arguments?.text ?? "")
      await writeFile(homeMcpMarker, `HOSTED_HOME_MCP_EXECUTED:${text}`, "utf8")
      return res.end(JSON.stringify({
        jsonrpc: "2.0",
        id: rpc.id ?? null,
        result: {
          content: [{ type: "text", text: JSON.stringify({ origin: "hosted-home-mcp", text }) }],
        },
      }))
    }
    res.end(JSON.stringify({
      jsonrpc: "2.0",
      id: rpc.id ?? null,
      error: { code: -32601, message: `unsupported MCP method ${rpc.method}` },
    }))
  })
  await new Promise((resolve, reject) => {
    homeOnlyMcp.once("error", reject)
    homeOnlyMcp.listen(homeOnlyMcpPort, "127.0.0.1", resolve)
  })

  return {
    homeMarker,
    homeMcpMarker,
    homeConnectorMarker,
    scriptPath,
    close: () => new Promise((resolve) => homeOnlyMcp.close(resolve)),
  }
}

async function registerHostedHomeExtensions({ client, requests, workspace, rootDir, python, fixtures, unwrap }) {
  const env = unwrap(await client.send(requests.registerEnvironmentRequest(workspace, {
    name: "hosted-home-python",
    runtime: { type: "python", python },
  })), "EnvironmentRegistered").environment
  await client.send(requests.registerScriptRequest(workspace, fixtures.scriptPath, env.name, "hosted_home_only_lookup"))

  const connectorAdapterDir = path.join(rootDir, "hosted-home-connector-adapter")
  await mkdir(connectorAdapterDir, { recursive: true })
  const connectorAdapterScript = path.join(connectorAdapterDir, "hosted_home_connector_adapter.mjs")
  await writeFile(connectorAdapterScript, `
import { appendFileSync, writeFileSync } from 'node:fs'
import readline from 'node:readline'

const marker = process.argv[2]
const rl = readline.createInterface({ input: process.stdin })
rl.on('line', (line) => {
  const request = JSON.parse(line)
  if (request.type === 'shutdown') process.exit(0)
  if (request.type === 'validate') {
    console.log(JSON.stringify({ id: request.id, ok: true }))
    return
  }
  if (request.type === 'prepare') {
    console.log(JSON.stringify({ id: request.id, ok: true, result: { credential_targets: [], prepared_config: { arguments: request.arguments ?? {}, config: request.config ?? {} } } }))
    return
  }
  if (request.type === 'call') {
    const q = String(request.config?.arguments?.q ?? '')
    writeFileSync(marker, 'HOSTED_HOME_CONNECTOR_EXECUTED:' + q, 'utf8')
    console.log(JSON.stringify({ id: request.id, ok: true, result: { origin: 'hosted-home-connector', q } }))
    return
  }
  appendFileSync(marker + '.errors', 'unsupported request ' + request.type + '\\n')
  console.log(JSON.stringify({ id: request.id, ok: false, error: 'unsupported request ' + request.type }))
})
`, "utf8")
  const connectorAdapterPath = path.join(connectorAdapterDir, "adapter.yaml")
  const connectorPath = path.join(rootDir, "hosted-home-local-api-connector.yaml")
  await writeFile(connectorAdapterPath, `
kind: connector_adapter
name: hosted_home_stub
version: 0.1.0
adapter_protocol: arroba-connector-adapter-v2
command: ${process.execPath}
args:
  - ${connectorAdapterScript}
  - ${fixtures.homeConnectorMarker}
description: Hosted home-only connector adapter for remote extension drill.
`, "utf8")
  await writeFile(connectorPath, `
kind: connector
name: hosted_home_local_api
description: Hosted home-only HTTP connector for remote extension drill.
adapter: hosted_home_stub
credential:
  required: false
timeout_ms: 30000
max_response_bytes: 1048576
operations:
  - name: public_echo
    description: Read hosted home-only connector echo data.
    safety: read
    input_schema:
      type: object
      required: [q]
      properties:
        q: { type: string }
      additionalProperties: false
    config:
      marker: ${fixtures.homeConnectorMarker}
`, "utf8")
  await client.send(requests.registerConnectorAdapterRequest(connectorAdapterPath))
  await client.send(requests.registerConnectorRequest(connectorPath))
  return env
}

async function grantHostedHomeExtensionAccess({ client, requests, workspace, agentId, env }) {
  await client.send(requests.grantAgentExtensionRequest(workspace, agentId, "script", "hosted_home_only_lookup", env.name))
  await client.send(requests.grantAgentExtensionRequest(workspace, agentId, "mcp", "hosted_home_echo_mcp"))
  await client.send(requests.grantAgentExtensionRequest(workspace, agentId, "connector", "hosted_home_local_api", null, { maxSafety: "read" }))
}

async function assertHostedHomeExtensionProxy({
  client,
  requests,
  workspace,
  sessionId,
  agentId,
  launch,
  env,
  fixtures,
  label = "hosted",
}) {
  if (!launch.runtime_mcp_server_url || !launch.runtime_mcp_auth_token) {
    throw new Error(`launched run lacks runtime MCP binding: ${JSON.stringify(launch)}`)
  }
  await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "hosted_home_only_lookup", true)
  const scriptQuery = `${label}-remote-agent`
  const scriptCall = await callRuntimeMcp(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "tools/call", {
    name: "hosted_home_only_lookup",
    arguments: { query: scriptQuery },
  })
  if (scriptCall.isError) throw new Error(`hosted home proxy script returned error: ${JSON.stringify(scriptCall)}`)
  const scriptMarker = await readFile(fixtures.homeMarker, "utf8")
  if (scriptMarker !== `HOSTED_HOME_SCRIPT_EXECUTED:${scriptQuery}`) {
    throw new Error(`hosted home script marker mismatch: ${JSON.stringify(scriptMarker)}`)
  }

  const proxyUrl = launch.runtime_mcp_server_url.replace(/\/mcp\/?$/, "/mcp/proxy/hosted_home_echo_mcp")
  const mcpTools = await callRuntimeMcp(proxyUrl, launch.runtime_mcp_auth_token, "tools/list")
  if (!mcpTools.tools.some((tool) => tool.name === "hosted_home_echo")) {
    throw new Error(`hosted home MCP tool not listed: ${JSON.stringify(mcpTools)}`)
  }
  const mcpText = `${label}-remote-mcp`
  const mcpCall = await callRuntimeMcp(proxyUrl, launch.runtime_mcp_auth_token, "tools/call", {
    name: "hosted_home_echo",
    arguments: { text: mcpText },
  })
  if (mcpCall.isError) throw new Error(`hosted home MCP returned error: ${JSON.stringify(mcpCall)}`)
  const mcpMarker = await readFile(fixtures.homeMcpMarker, "utf8")
  if (mcpMarker !== `HOSTED_HOME_MCP_EXECUTED:${mcpText}`) {
    throw new Error(`hosted home MCP marker mismatch: ${JSON.stringify(mcpMarker)}`)
  }

  await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "hosted_home_local_api_public_echo", true)
  const connectorQuery = `${label}-remote-connector`
  const connectorCall = await callRuntimeMcp(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "tools/call", {
    name: "hosted_home_local_api_public_echo",
    arguments: { q: connectorQuery },
  })
  if (connectorCall.isError) throw new Error(`hosted home connector returned error: ${JSON.stringify(connectorCall)}`)
  const connectorMarker = await readFile(fixtures.homeConnectorMarker, "utf8")
  if (connectorMarker !== `HOSTED_HOME_CONNECTOR_EXECUTED:${connectorQuery}`) {
    throw new Error(`hosted home connector marker mismatch: ${JSON.stringify(connectorMarker)}`)
  }

  await client.send(requests.revokeAgentExtensionRequest(agentId, "script", "hosted_home_only_lookup"))
  await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "hosted_home_only_lookup", false)
  await expectRuntimeMcpReject(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "tools/call", {
    name: "hosted_home_only_lookup",
    arguments: { query: "hosted-after-revoke" },
  })
  const afterRevokeMarker = await readFile(fixtures.homeMarker, "utf8")
  if (afterRevokeMarker !== scriptMarker) throw new Error("revoked hosted home-proxy script executed after revoke")
  await client.send(requests.revokeAgentExtensionRequest(agentId, "mcp", "hosted_home_echo_mcp"))
  await expectRuntimeMcpReject(proxyUrl, launch.runtime_mcp_auth_token, "tools/call", {
    name: "hosted_home_echo",
    arguments: { text: "hosted-after-mcp-revoke" },
  })
  const afterMcpRevokeMarker = await readFile(fixtures.homeMcpMarker, "utf8")
  if (afterMcpRevokeMarker !== mcpMarker) throw new Error("revoked hosted home-proxy MCP executed after revoke")
  await client.send(requests.revokeAgentExtensionRequest(agentId, "connector", "hosted_home_local_api"))
  await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "hosted_home_local_api_public_echo", false)
  await expectRuntimeMcpReject(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "tools/call", {
    name: "hosted_home_local_api_public_echo",
    arguments: { q: "hosted-after-connector-revoke" },
  })
  const afterConnectorRevokeMarker = await readFile(fixtures.homeConnectorMarker, "utf8")
  if (afterConnectorRevokeMarker !== connectorMarker) throw new Error("revoked hosted home-proxy connector executed after revoke")

  return { sessionId, agentId }
}

async function assertHostedCollaboratorHomeExtensions({
  LocalIpcClient,
  requests,
  homeClient,
  ownerProfile,
  ownerClientId,
  homeDaemonAlias,
  workspace,
  workerWorkspace,
  liveSyncFixture = null,
  session,
  workerDaemonId,
  env,
  fixtures,
  apiUrl,
  log,
  assert,
  unwrap,
  postJson,
  issueSessionScopedClientToken,
  pollTimeoutMs,
  devBrowserCloudLogin,
  installSendRetry,
  expectReject,
  cleanupHostedCloudIdentity,
}) {
  log("second-kernel-collab-extension-start", { sessionId: session.id })
  const localInvite = unwrap(
    await homeClient.send(requests.createSessionInviteRequest(session.id, null, 1, "full")),
    "SessionInviteCreated",
  )
  const cloudInvite = unwrap(
    await homeClient.send(requests.createCloudSessionInviteRequest(session.id, {
      displayName: "Hosted home extension collab drill",
      maxUses: 1,
    })),
    "CloudSessionInviteCreated",
  )
  const localInviteToken = localInvite.invite?.invite_token
  const cloudInviteToken = cloudInvite.invite?.invite_token
  assert(localInviteToken, "hosted extension collab local invite token should be returned", localInvite)
  assert(cloudInviteToken, "hosted extension collab cloud invite token should be returned", cloudInvite)

  const peerClientId = `${ownerClientId}-extension-peer-${Date.now()}`
  const ownerScopedToken = await issueSessionScopedClientToken(apiUrl, {
    sessionToken: ownerProfile.cloudSessionToken,
    accountId: ownerProfile.accountId,
    realmId: ownerProfile.realmId,
    subject: ownerClientId,
    userId: ownerProfile.userId,
    clientId: ownerClientId,
    sessionId: session.id,
    targetDaemonAlias: homeDaemonAlias,
  })
  const ownerScopedClient = installSendRetry(new LocalIpcClient(ownerProfile.relayUrl, {
    relayAuthToken: ownerScopedToken,
    targetDaemonAlias: homeDaemonAlias,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  }), "extension-owner-relay")
  let peerRemoteClient = null
  let peerLogin = null
  let scenarioFailure = null
  try {
    peerLogin = await devBrowserCloudLogin({ role: "extension-peer" })
    const peerProfile = peerLogin.profile
    assert(peerProfile.userId !== ownerProfile.userId, "extension peer login must use a different cloud user", {
      ownerUserId: ownerProfile.userId,
      peerUserId: peerProfile.userId,
    })
    const peerAcceptance = await postJson(`${apiUrl}/sessions/invites/${encodeURIComponent(cloudInviteToken)}/accept`, {
      sessionToken: peerLogin.cloudSessionToken,
    })
    assert(peerAcceptance.userId === peerProfile.userId, "extension peer should accept cloud invite as itself", peerAcceptance)
    const peerRelayToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: peerLogin.cloudSessionToken,
      accountId: ownerProfile.accountId,
      realmId: ownerProfile.realmId,
      subject: `browser:${peerProfile.userId}:${Date.now()}`,
      userId: peerProfile.userId,
      clientId: peerClientId,
      sessionId: session.id,
      targetDaemonAlias: homeDaemonAlias,
      allowUnpairedClientSubject: true,
    })
    peerRemoteClient = installSendRetry(new LocalIpcClient(ownerProfile.relayUrl, {
      relayAuthToken: peerRelayToken,
      targetDaemonAlias: homeDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    }), "extension-peer-relay")
    await peerRemoteClient.send(requests.joinSessionInviteRequest(localInviteToken, peerProfile.userId))
    const peerAttachment = unwrap(
      await peerRemoteClient.send(requests.attachToSessionRequest(session.id, `hosted-extension-peer-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    const agent = unwrap(
      await peerRemoteClient.send(requests.spawnAgentRequest(
        session.id,
        "dev-stub",
        "hosted-extension-peer-agent",
        HOSTED_HOME_PROXY_MODEL,
        workerWorkspace,
        "low",
        undefined,
        undefined,
        workerDaemonId,
      )),
      "AgentSpawned",
    ).agent
    assert(agent.owner_user_id === peerProfile.userId, "hosted extension peer agent should be owned by peer user", agent)
    await expectReject(
      peerRemoteClient.send(requests.grantAgentExtensionRequest(workspace, agent.id, "script", "hosted_home_only_lookup", env.name)),
      "hosted extension peer granting home script",
      "home extensions for remote-backed agent",
    )
    await grantHostedHomeExtensionAccess({
      client: ownerScopedClient,
      requests,
      workspace,
      agentId: agent.id,
      env,
    })
    const launch = await startHostedRemoteProviderRun({
      client: peerRemoteClient,
      requests,
      sessionId: session.id,
      attachmentId: peerAttachment.id,
      agentId: agent.id,
      prompt: "Initialize the hosted collaborator home-proxy extension runtime.",
      timeoutMs: pollTimeoutMs,
    })
    const deniedRequest = await callRuntimeMcp(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, "tools/call", {
      name: "arroba.request_extension",
      arguments: { kind: "script", name: "hosted_home_only_lookup", environment: env.name },
    }, { retryTransient: true })
    if (!deniedRequest.isError || !JSON.stringify(deniedRequest).includes("home-owned extensions for collaborator remote agents")) {
      throw new Error(`hosted extension collaborator request_extension returned unexpected result: ${JSON.stringify(deniedRequest)}`)
    }
    if (liveSyncFixture) {
      await assertHostedWorkspaceLiveSyncProxy({
        client: ownerScopedClient,
        requests,
        session,
        launch,
        workerWorkspace,
        targetWorkspace: liveSyncFixture.targetWorkspace,
        pollTimeoutMs,
        label: "collab",
        log,
        unwrap,
      })
    }
    await expectReject(
      peerRemoteClient.send(requests.revokeAgentExtensionRequest(agent.id, "mcp", "hosted_home_echo_mcp")),
      "hosted extension peer revoking home MCP",
      "home extensions for remote-backed agent",
    )
    await assertHostedHomeExtensionProxy({
      client: ownerScopedClient,
      requests,
      workspace,
      sessionId: session.id,
      agentId: agent.id,
      launch,
      env,
      fixtures,
      label: "collab",
    })
    await peerRemoteClient.send(requests.cancelActivePromptRequest(
      session.id,
      peerAttachment.id,
      agent.id,
    ))
    log("second-kernel-collab-extension-pass", {
      sessionId: session.id,
      agentId: agent.id,
      peerUserId: peerProfile.userId,
      workspaceLiveSync: Boolean(liveSyncFixture),
    })
  } catch (error) {
    scenarioFailure = error
    throw error
  } finally {
    await peerRemoteClient?.close().catch(() => {})
    await ownerScopedClient.close().catch(() => {})
    if (peerLogin) {
      await cleanupHostedCloudIdentity({
        profile: peerLogin.profile,
        cloudSessionToken: peerLogin.cloudSessionToken,
        reason: "hosted Cloud extension collaborator cleanup",
      }).catch((error) => {
        log("second-kernel-collab-cleanup-failed", {
          error: error instanceof Error ? error.message : String(error),
        })
        if (!scenarioFailure) throw error
      })
    }
  }
}

export async function runHostedSecondKernelAssertions({
  LocalIpcClient,
  requests,
  kernelPath,
  rootDir,
  workspace,
  session: existingSession = null,
  kernelUrl,
  homeHistoryDir,
  python,
  homeCapabilityRoot,
  homeOnlyMcpPort,
  collabExtensions = false,
  workspaceLiveSync = false,
  trackedWorkspaceLiveSync = false,
  trackedWorkspaceLiveSyncProvider = "codex",
  trackedWorkspaceLiveSyncModel = "gpt-5.2",
  homeDaemonAlias,
  homeClient,
  ownerProfile,
  ownerClientId,
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
  postJson,
  manualCloudDeviceLogin,
  devBrowserCloudLogin,
  installSendRetry,
  expectReject,
  waitForLocalDaemon,
  allowDevStubProvider,
  waitForRelayTarget,
  waitForRemoteMachine,
  waitForCompletion,
  closeClient,
  terminateChild,
  spawnProcess,
  cleanupHostedCloudIdentity,
}) {
  const workerPorts = await makeWorkerPorts()
  const workerDaemonId = `hosted-worker-daemon-${process.pid}-${Date.now()}`
  const workerAlias = `hosted-worker-${process.pid}`
  const workerHome = path.join(rootDir, "worker-home")
  const workerArrobaHome = path.join(workerHome, ".arroba")
  const workerCapabilityRoot = path.join(rootDir, "worker-capabilities")
  const workerWorkspace = path.join(rootDir, "worker-workspace")
  const workerHistoryDir = path.join(rootDir, "worker-session-history")

  let worker = null
  let workerClient = null
  let fixtures = null
  let workerMachinePaired = false
  let scenarioFailure = null
  const eventLog = []
  try {
    log("second-kernel-cloud-pair-machine", { machineId: workerDaemonId, alias: workerAlias })
    await pairCloudMachineDirect({
      profile: ownerProfile,
      machineId: workerDaemonId,
      alias: workerAlias,
    })
    workerMachinePaired = true
    const workerRelayToken = await issueMachineRelayToken({
      profile: ownerProfile,
      machineId: workerDaemonId,
    })
    await mkdir(workerArrobaHome, { recursive: true })
    await mkdir(workerWorkspace, { recursive: true })
    const workerEnv = withDevStubProviderInventory({
      ...process.env,
      HOME: workerHome,
      ARROBA_HOME: workerArrobaHome,
      ARROBA_KERNEL_PORT: String(workerPorts.kernelPort),
      ARROBA_MCP_PORT: String(workerPorts.mcpPort),
      ARROBA_OPENCODE_PORT: String(workerPorts.opencodePort),
      ARROBA_CODEX_PORT: String(workerPorts.codexPort),
      ARROBA_RELAY_URL: ownerProfile.relayUrl,
      ARROBA_RELAY_TOKEN: workerRelayToken,
      ARROBA_DAEMON_ID: workerDaemonId,
      ARROBA_DAEMON_ALIAS: workerAlias,
      ARROBA_MACHINE_ID: workerDaemonId,
      ARROBA_MACHINE_ALIAS: workerAlias,
      ARROBA_ACCEPT_REMOTE_LEASES: "1",
      ARROBA_DAEMON_SOCKET: path.join(rootDir, "worker-daemon.sock"),
      ARROBA_SESSION_HISTORY_DIR: workerHistoryDir,
      ARROBA_CAPABILITY_ISOLATION_ROOT: workerCapabilityRoot,
      ...(trackedWorkspaceLiveSyncProvider === "codex"
        ? { CODEX_HOME: process.env.CODEX_HOME?.trim() || path.join(process.env.HOME ?? "", ".codex") }
        : {}),
    })
    fixtures = await createHostedHomeExtensionFixtures({ rootDir, homeCapabilityRoot, homeOnlyMcpPort })
    log("start-second-kernel", { workerAlias })
    worker = spawnProcess(kernelPath, [], { cwd: repoRoot, env: workerEnv, name: "worker-kernel" })
    const workerKernelUrl = `ws://127.0.0.1:${workerPorts.kernelPort}/kernel`
    await waitForLocalDaemon(LocalIpcClient, requests, workerKernelUrl, workerWorkspace)
    workerClient = new LocalIpcClient(workerKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await allowDevStubProvider(homeClient, requests, "second-kernel-home")
    await allowDevStubProvider(workerClient, requests, "second-kernel-worker")
    const env = await registerHostedHomeExtensions({
      client: homeClient,
      requests,
      workspace,
      rootDir,
      python,
      fixtures,
      unwrap,
    })

    log("second-kernel-client-token-request", { workerAlias })
    const workerClientToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: ownerProfile.cloudSessionToken,
      accountId: ownerProfile.accountId,
      realmId: ownerProfile.realmId,
      subject: ownerClientId,
      userId: ownerProfile.userId,
      clientId: ownerClientId,
      targetDaemonAlias: workerAlias,
    })
    log("second-kernel-client-token-issued", { workerAlias })
    log("second-kernel-relay-target-probe", { workerAlias })
    await waitForRelayTarget(
      LocalIpcClient,
      requests,
      ownerProfile.relayUrl,
      workerClientToken,
      workerAlias,
    )
    log("second-kernel-relay-target-ready", { workerAlias })

    await waitForRemoteMachine(homeClient, requests, workerDaemonId)
    const ownsSession = existingSession == null
    const session = existingSession ?? unwrap(
      await homeClient.send(requests.createSessionRequest(workspace, workspace)),
      "SessionCreated",
    ).session
    const attachment = unwrap(
      await homeClient.send(requests.attachToSessionRequest(session.id, `hosted-second-kernel-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    homeClient.onKernelEvent((event) => {
      eventLog.push({ ...event, observed_at_ms: Date.now() })
    })
    log("second-kernel-subscribe-start", { sessionId: session.id, attachmentId: attachment.id })
    await homeClient.subscribeToKernelEvents(session.id, attachment.id)
    log("second-kernel-subscribe-ready", { sessionId: session.id, attachmentId: attachment.id })

    const liveSyncFixture = workspaceLiveSync
      ? await prepareHostedWorkspaceLiveSyncFixture({
          client: homeClient,
          requests,
          session,
          workspace,
          workerWorkspace,
          rootDir,
          log,
          unwrap,
        })
      : null

    log("second-kernel-spawn-agent-start", { workerDaemonId })
    const spawned = unwrap(
      await homeClient.send(requests.spawnAgentRequest(
        session.id,
        "dev-stub",
        "hosted-worker-agent",
        HOSTED_HOME_PROXY_MODEL,
        workerWorkspace,
        "low",
        undefined,
        undefined,
        workerDaemonId,
      )),
      "AgentSpawned",
    )
    log("second-kernel-spawn-agent-ready", { workerDaemonId, agentId: spawned.agent?.id })
    assert(spawned.agent?.remote_execution?.worker_machine_id === workerDaemonId, "remote dev-stub agent should be leased to the second kernel", spawned)
    await grantHostedHomeExtensionAccess({
      client: homeClient,
      requests,
      workspace,
      agentId: spawned.agent.id,
      env,
    })
    const launch = await startHostedRemoteProviderRun({
      client: homeClient,
      requests,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: spawned.agent.id,
      prompt: "Initialize the hosted home-proxy extension runtime.",
      timeoutMs: pollTimeoutMs,
    })
    if (liveSyncFixture) {
      await assertHostedWorkspaceLiveSyncProxy({
        client: homeClient,
        requests,
        session,
        launch,
        workerWorkspace,
        targetWorkspace: liveSyncFixture.targetWorkspace,
        pollTimeoutMs,
        log,
        unwrap,
      })
    }
    await assertHostedHomeExtensionProxy({
      client: homeClient,
      requests,
      workspace,
      sessionId: session.id,
      agentId: spawned.agent.id,
      launch,
      env,
      fixtures,
      label: "single",
    })
    let completed = null
    try {
      completed = unwrap(
        await homeClient.send(requests.completePromptRequest(session.id)),
        "PromptCompleted",
      )
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      if (!message.includes("has no active prompt")) throw error
      log("second-kernel-complete-prompt-already-settled", { sessionId: session.id })
    }
    await waitForCompletion(eventLog, pollTimeoutMs, 0)
    if (collabExtensions) {
      await assertHostedCollaboratorHomeExtensions({
        LocalIpcClient,
        requests,
        homeClient,
        ownerProfile,
        ownerClientId,
        homeDaemonAlias,
        workspace,
        workerWorkspace,
        liveSyncFixture,
        session,
        workerDaemonId,
        env,
        fixtures,
        apiUrl,
        log,
        assert,
        unwrap,
        postJson,
        issueSessionScopedClientToken,
        pollTimeoutMs,
        devBrowserCloudLogin,
        installSendRetry,
        expectReject,
        cleanupHostedCloudIdentity,
      })
    }
    if (trackedWorkspaceLiveSync) {
      if (!kernelUrl || !homeHistoryDir) {
        throw new Error("hosted tracked workspace live sync drill requires kernelUrl and homeHistoryDir")
      }
      await runHostedTrackedWorkspaceLiveSyncProviderDrill({
        rootDir,
        repoRoot,
        kernelUrl,
        workerDaemonId,
        homeHistoryDir,
        workerHistoryDir,
        provider: trackedWorkspaceLiveSyncProvider,
        model: trackedWorkspaceLiveSyncModel,
        timeoutMs: pollTimeoutMs,
        pollMs: 1_000,
        log,
      })
    }
    if (ownsSession) {
      await homeClient.send(requests.endSessionRequest(session.id)).catch(() => {})
    }
    log("second-kernel-pass", {
      machineId: workerDaemonId,
      workerAlias,
      agentId: spawned.agent.id,
      completedPromptId: completed?.completion?.completed?.id ?? null,
      homeExtensions: ["script", "mcp", "connector"],
      workspaceLiveSync,
    })
  } catch (error) {
    scenarioFailure = error
    throw error
  } finally {
    await fixtures?.close?.().catch(() => {})
    await closeClient(workerClient, "worker")
    await terminateChild(worker)
    if (workerMachinePaired) {
      await cleanupHostedCloudIdentity({
        profile: ownerProfile,
        machineIds: [workerDaemonId],
        reason: "hosted Cloud worker drill cleanup",
        logout: false,
      }).catch((error) => {
        log("second-kernel-cloud-cleanup-failed", {
          error: error instanceof Error ? error.message : String(error),
        })
        if (!scenarioFailure) throw error
      })
    }
  }
}

export async function runHostedTokenRotationAssertions({
  requests,
  homeClient,
  verificationClient,
  sessionId,
  log,
  assert,
  unwrap,
}) {
  log("token-rotation-start", { sessionId })
  const assertSessionReachable = async (label) => {
    const listed = unwrap(
      await verificationClient.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    assert(
      (listed.sessions ?? []).some((session) => session.id === sessionId),
      `token rotation probe should list session during ${label}`,
      listed,
    )
  }

  await assertSessionReachable("before-rotation")
  let probeCount = 0
  let probeFailure = null
  const probeUntilMs = Date.now() + 15_000
  const probeTask = (async () => {
    while (Date.now() < probeUntilMs) {
      try {
        await assertSessionReachable("rotation")
        probeCount += 1
      } catch (error) {
        probeFailure = error
        break
      }
      await sleep(100)
    }
  })()

  await sleep(500)
  const rotated = unwrap(
    await homeClient.send(requests.connectCloudRelayRequest()),
    "CloudRelayConnected",
  )
  log("token-rotation-issued", {
    tokenExpiresAt: rotated.token?.token_expires_at ?? null,
  })
  await probeTask
  if (probeFailure) {
    throw probeFailure
  }
  await assertSessionReachable("after-rotation")
  log("token-rotation-pass", { probeCount })
}
