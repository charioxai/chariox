import { rm } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"

async function waitForLocalSocket(socketPath, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const socket = net.createConnection(socketPath)
      await new Promise((resolve, reject) => {
        socket.once("connect", resolve)
        socket.once("error", reject)
      })
      socket.destroy()
      return
    } catch (error) {
      lastError = error
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`local automation socket did not become ready: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

function createAutomationClient(socketPath) {
  const socket = net.createConnection(socketPath)
  socket.setEncoding("utf8")
  let nextId = 1
  let buffer = ""
  const pending = new Map()
  socket.on("data", (chunk) => {
    buffer += chunk
    while (buffer.includes("\n")) {
      const newline = buffer.indexOf("\n")
      const line = buffer.slice(0, newline).trim()
      buffer = buffer.slice(newline + 1)
      if (!line) continue
      const response = JSON.parse(line)
      const deferred = pending.get(response.id)
      if (!deferred) continue
      pending.delete(response.id)
      if (response.ok) deferred.resolve(response.data)
      else deferred.reject(new Error(response.error ?? "automation command failed"))
    }
  })
  socket.on("error", (error) => {
    for (const deferred of pending.values()) deferred.reject(error)
    pending.clear()
  })
  return {
    send(action, fields = {}) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(`${JSON.stringify({ id, action, ...fields })}\n`)
      })
    },
    close() {
      socket.destroy()
    },
  }
}

async function waitForRemoteSocket(socketPath, { runSsh, shellQuote }, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    const result = await runSsh(`[ -S ${shellQuote(socketPath)} ]`)
    if (result.code === 0) return
    lastError = result.stderr || result.stdout || `exit ${result.code}`
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`remote automation socket did not become ready: ${lastError}`)
}

async function remoteAutomation(socketPath, action, fields = {}, { runSsh, shellQuote, assert }) {
  const request = JSON.stringify({ id: 1, action, ...fields })
  const code = `
const net = require("node:net");
const socketPath = process.argv[1];
const request = JSON.parse(process.argv[2]);
const socket = net.createConnection(socketPath);
socket.setEncoding("utf8");
let buffer = "";
socket.on("data", (chunk) => {
  buffer += chunk;
  const newline = buffer.indexOf("\\n");
  if (newline === -1) return;
  console.log(buffer.slice(0, newline));
  socket.end();
});
socket.on("error", (error) => {
  console.error(error.message);
  process.exit(1);
});
socket.write(JSON.stringify(request) + "\\n");
`
  const result = await runSsh(
    `export PATH=/root/.bun/bin:/opt/node-v22/bin:$PATH; node -e ${shellQuote(code)} ${shellQuote(socketPath)} ${shellQuote(request)}`,
  )
  if (result.code !== 0) {
    throw new Error(`remote automation ${action} failed\n${result.stdout}\n${result.stderr}`)
  }
  const line = result.stdout.trim().split("\n").filter(Boolean).at(-1)
  assert(line, `remote automation ${action} should return a response`, result)
  const response = JSON.parse(line)
  if (!response.ok) {
    throw new Error(`remote automation ${action} rejected: ${response.error ?? "unknown error"}`)
  }
  return response.data
}

async function waitForCliExit(child, label, timeoutMs = 15_000) {
  if (!child) throw new Error(`${label} process was not started`)
  if (child.exitCode != null || child.signalCode != null) {
    if (child.exitCode === 0) return
    throw new Error(`${label} exited with ${child.signalCode ?? child.exitCode}`)
  }
  const result = await Promise.race([
    new Promise((resolve) => {
      child.once("exit", (code, signal) => resolve({ code, signal }))
    }),
    new Promise((resolve) => setTimeout(() => resolve(null), timeoutMs)),
  ])
  if (!result) {
    throw new Error(`${label} did not exit within ${timeoutMs}ms after automation exit`)
  }
  if (result.code !== 0) {
    throw new Error(`${label} exited with ${result.signal ?? result.code}`)
  }
}

async function endSessionIfPresent(client, requests, sessionId) {
  try {
    const listed = await client.send(requests.listSessionsRequest())
    const sessions = listed?.SessionsListed?.sessions ?? listed?.sessions ?? []
    if (sessions.some((session) => session.id === sessionId)) {
      await client.send(requests.endSessionRequest(sessionId)).catch(() => {})
    }
  } catch {}
}

export async function runHostedRemoteCliPairingAssertions({
  requests,
  homeClient,
  verificationClient,
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
}) {
  const remoteId = `${process.pid}-${Date.now()}`
  const localAlias = `hosted-pairing-local-cli-${remoteId}`
  const remoteAlias = `hosted-pairing-cli-${remoteId}`
  const remoteWorkspace = `/tmp/arroba-hosted-pairing-cli-${remoteId}`
  const localSocket = path.join(os.tmpdir(), `arroba-hosted-local-cli-${remoteId}.sock`)
  const remoteSocket = `/tmp/arroba-hosted-pairing-cli-${remoteId}.sock`
  const localMarker = `HOSTED_PAIRING_LOCAL_CLI_OK_${remoteId.replace(/[^a-zA-Z0-9]/g, "_")}`
  const remoteMarker = `HOSTED_PAIRING_REMOTE_CLI_OK_${remoteId.replace(/[^a-zA-Z0-9]/g, "_")}`
  const pairing = unwrap(
    await homeClient.send(requests.createTerminalPairingLinkRequest("cli", remoteAlias, 15 * 60 * 1000)),
    "TerminalPairingLinkCreated",
  ).pairing
  assert(pairing?.pairing_link, "terminal pairing link should be created", pairing)
  assert(pairing.terminal_id, "terminal pairing should include terminal id", pairing)

  const remoteCommand = [
    "set -e",
    "export PATH=/root/.bun/bin:/opt/node-v22/bin:$PATH",
    "export ARROBA_TEST_TUI=1",
    `mkdir -p ${shellQuote(remoteWorkspace)}`,
    `cd ${shellQuote(path.posix.join(remoteCliRepo, "apps/cli"))}`,
    [
      "bun",
      "dist/index.js",
      "--terminal-pairing-link",
      shellQuote(pairing.pairing_link),
      "--automation-socket",
      shellQuote(remoteSocket),
      "--create-session",
      "--alias",
      shellQuote(remoteAlias),
      "--workspace",
      shellQuote(workspace),
      "--worktree",
      shellQuote(workspace),
      "--provider",
      shellQuote(remoteCliPairingProvider),
      "--model",
      shellQuote(remoteCliPairingModel),
      "--effort",
      shellQuote(remoteCliPairingEffort),
    ].join(" "),
  ].join("; ")

  let localCli = null
  let localAutomation = null
  let remoteCli = null
  try {
    log("local-cli-pairing-start", {
      alias: localAlias,
      provider: remoteCliPairingProvider,
      model: remoteCliPairingModel,
    })
    localCli = spawnProcess("script", [
      "-q",
      "/dev/null",
      "env",
      "ARROBA_TEST_TUI=1",
      "bun",
      path.join(cliRoot, "dist/index.js"),
      "--kernel-url",
      kernelUrl,
      "--automation-socket",
      localSocket,
      "--create-session",
      "--alias",
      localAlias,
      "--workspace",
      workspace,
      "--worktree",
      workspace,
      "--provider",
      remoteCliPairingProvider,
      "--model",
      remoteCliPairingModel,
      "--effort",
      remoteCliPairingEffort,
      "--client-id",
      `hosted-local-pairing-cli-${remoteId}`,
    ], {
      cwd: repoRoot,
      env: process.env,
      name: "local-cli-pairing",
      logStdout: false,
    })
    await waitForLocalSocket(localSocket)
    localAutomation = createAutomationClient(localSocket)
    await localAutomation.send("ping")
    const localSnapshot = await localAutomation.send("wait_for", { screen: "agents", timeoutMs: 10_000 })
    assert(localSnapshot.session?.id, "local TUI should create and attach to a session", localSnapshot)
    assert(
      localSnapshot.session?.focusedAgentId,
      "local TUI should create a focused real-provider agent",
      localSnapshot,
    )

    log("remote-cli-pairing-start", {
      host: remoteCliHost,
      repo: remoteCliRepo,
      alias: remoteAlias,
      terminalId: pairing.terminal_id,
      provider: remoteCliPairingProvider,
      model: remoteCliPairingModel,
    })
    remoteCli = spawnProcess("ssh", sshArgs(remoteCommand, { tty: true }), {
      cwd: repoRoot,
      env: process.env,
      name: "remote-cli-pairing",
      logStdout: false,
    })
    try {
      await waitForRemoteSocket(remoteSocket, { runSsh, shellQuote })
    } catch (error) {
      throw new Error(`${error instanceof Error ? error.message : String(error)}\nremote alias: ${remoteAlias}`)
    }

    await remoteAutomation(remoteSocket, "ping", {}, { runSsh, shellQuote, assert })
    const remoteSnapshot = await remoteAutomation(remoteSocket, "wait_for", { screen: "agents", timeoutMs: 10_000 }, { runSsh, shellQuote, assert })
    assert(remoteSnapshot.session?.id, "paired orphan CLI should attach to a session", remoteSnapshot)
    assert(
      remoteSnapshot.session?.focusedAgentId,
      "paired orphan CLI should create a focused real-provider agent",
      remoteSnapshot,
    )

    const terminals = unwrap(
      await homeClient.send(requests.listTerminalsRequest()),
      "TerminalsListed",
    ).terminals ?? []
    assert(
      terminals.some((terminal) => terminal.terminal_id === pairing.terminal_id && terminal.terminal_type === "cli"),
      "home kernel should list the paired CLI terminal",
      { pairing, terminals },
    )

    await waitForSession(homeClient, requests, localSnapshot.session.id)
    await waitForSession(verificationClient, requests, localSnapshot.session.id)
    await waitForSession(homeClient, requests, remoteSnapshot.session.id)
    await waitForSession(verificationClient, requests, remoteSnapshot.session.id)

    const listed = unwrap(
      await verificationClient.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    const localSession = listed.sessions?.find((session) => session.id === localSnapshot.session.id)
    const remoteSession = listed.sessions?.find((session) => session.alias === remoteAlias)
    assert(localSession, "hosted relay client should list the session created by the local TUI", {
      localAlias,
      sessions: listed.sessions,
    })
    assert(remoteSession, "home kernel should list the session created by the paired orphan CLI", {
      remoteAlias,
      sessions: listed.sessions,
    })
    assert(remoteSession.id === remoteSnapshot.session.id, "paired CLI snapshot should match home kernel session", {
      snapshotSession: remoteSnapshot.session,
      remoteSession,
    })

    const localAgents = unwrap(
      await verificationClient.send(requests.listAgentsRequest(localSession.id)),
      "AgentsListed",
    ).agents ?? []
    const localFocusedAgent = localAgents.find((agent) => agent.id === localSnapshot.session.focusedAgentId)
    assert(
      localFocusedAgent && localFocusedAgent.provider === remoteCliPairingProvider && localFocusedAgent.provider !== "dev-stub",
      "local TUI should use the configured real provider, not dev-stub",
      { localFocusedAgent, localAgents, expectedProvider: remoteCliPairingProvider },
    )

    const remoteAgents = unwrap(
      await verificationClient.send(requests.listAgentsRequest(remoteSession.id)),
      "AgentsListed",
    ).agents ?? []
    const remoteFocusedAgent = remoteAgents.find((agent) => agent.id === remoteSnapshot.session.focusedAgentId)
    assert(
      remoteFocusedAgent && remoteFocusedAgent.provider === remoteCliPairingProvider && remoteFocusedAgent.provider !== "dev-stub",
      "paired orphan CLI should use the configured real provider, not dev-stub",
      { remoteFocusedAgent, remoteAgents, expectedProvider: remoteCliPairingProvider },
    )

    await localAutomation.send("submit_prompt", {
      prompt: `Reply with exactly ${localMarker} and nothing else.`,
    })
    await Promise.all([
      waitForHistoryText(homeClient, requests, localSession.id, localSnapshot.session.focusedAgentId, localMarker, pollTimeoutMs, undefined, { providerOutputOnly: true }),
      waitForHistoryText(verificationClient, requests, localSession.id, localSnapshot.session.focusedAgentId, localMarker, pollTimeoutMs, undefined, { providerOutputOnly: true }),
    ])

    await remoteAutomation(remoteSocket, "submit_prompt", {
      prompt: `Reply with exactly ${remoteMarker} and nothing else.`,
      timeoutMs: pollTimeoutMs,
    }, { runSsh, shellQuote, assert })
    await Promise.all([
      waitForHistoryText(homeClient, requests, remoteSession.id, remoteSnapshot.session.focusedAgentId, remoteMarker, pollTimeoutMs, undefined, { providerOutputOnly: true }),
      waitForHistoryText(verificationClient, requests, remoteSession.id, remoteSnapshot.session.focusedAgentId, remoteMarker, pollTimeoutMs, undefined, { providerOutputOnly: true }),
    ])

    await localAutomation.send("exit").catch(() => {})
    await remoteAutomation(remoteSocket, "exit", {}, { runSsh, shellQuote, assert }).catch(() => {})
    await waitForCliExit(localCli, "local paired CLI")
    await waitForCliExit(remoteCli, "remote paired CLI")
    await endSessionIfPresent(homeClient, requests, localSession.id)
    await endSessionIfPresent(homeClient, requests, remoteSession.id)
    log("remote-cli-pairing-pass", {
      host: remoteCliHost,
      localSessionId: localSession.id,
      remoteSessionId: remoteSession.id,
      localAlias,
      remoteAlias,
      terminalId: pairing.terminal_id,
      provider: remoteCliPairingProvider,
      model: remoteCliPairingModel,
      localMarker,
      remoteMarker,
    })
  } finally {
    localAutomation?.close()
    await terminateChild(localCli)
    await terminateChild(remoteCli)
    await rm(localSocket, { force: true }).catch(() => {})
    await runSsh(`rm -f ${shellQuote(remoteSocket)}; rm -rf ${shellQuote(remoteWorkspace)}`).catch(() => {})
  }
}

export async function runHostedRemoteCliAssertions({
  requests,
  homeClient,
  verificationClient,
  relayUrl,
  relayToken,
  targetDaemonAlias,
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
}) {
  const remoteId = `${process.pid}-${Date.now()}`
  const remoteAlias = `hosted-remote-cli-${remoteId}`
  const remoteClientId = `hosted-remote-client-${remoteId}`
  const remoteWorkspace = `/tmp/arroba-hosted-remote-cli-${remoteId}`
  const remoteSocket = `/tmp/arroba-hosted-remote-cli-${remoteId}.sock`
  const remoteCommand = [
    "set -e",
    "export PATH=/root/.bun/bin:/opt/node-v22/bin:$PATH",
    "export ARROBA_TEST_TUI=1",
    `mkdir -p ${shellQuote(remoteWorkspace)}`,
    `cd ${shellQuote(path.posix.join(remoteCliRepo, "apps/cli"))}`,
    [
      "bun",
      "dist/index.js",
      "--relay-url",
      shellQuote(relayUrl),
      "--relay-token",
      shellQuote(relayToken),
      "--target-daemon-alias",
      shellQuote(targetDaemonAlias),
      "--automation-socket",
      shellQuote(remoteSocket),
      "--create-session",
      "--alias",
      shellQuote(remoteAlias),
      "--workspace",
      shellQuote(remoteWorkspace),
      "--worktree",
      shellQuote(remoteWorkspace),
      "--client-id",
      shellQuote(remoteClientId),
      "--provider",
      "dev-stub",
      "--model",
      "remote-cli-drill",
      "--effort",
      "low",
    ].join(" "),
  ].join("; ")

  let remoteCli = null
  try {
    await allowDevStubProvider(homeClient, requests, "remote-cli-home-kernel")
    log("remote-cli-start", { host: remoteCliHost, repo: remoteCliRepo, alias: remoteAlias })
    remoteCli = spawnProcess("ssh", sshArgs(remoteCommand, { tty: true }), {
      cwd: repoRoot,
      env: process.env,
      name: "remote-cli",
    })
    try {
      await waitForRemoteSocket(remoteSocket, { runSsh, shellQuote })
    } catch (error) {
      throw new Error(`${error instanceof Error ? error.message : String(error)}\nremote alias: ${remoteAlias}`)
    }
    await remoteAutomation(remoteSocket, "ping", {}, { runSsh, shellQuote, assert })
    const snapshot = await remoteAutomation(remoteSocket, "wait_for", { screen: "agents", timeoutMs: 10_000 }, { runSsh, shellQuote, assert })
    assert(snapshot.session?.id, "remote CLI should attach to a session", snapshot)
    const listed = unwrap(
      await verificationClient.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    const remoteSession = listed.sessions?.find((session) => session.alias === remoteAlias)
    assert(remoteSession, "home kernel should list the session created by the remote CLI", {
      remoteAlias,
      sessions: listed.sessions,
    })
    assert(remoteSession.id === snapshot.session.id, "remote CLI snapshot should match home kernel session", {
      snapshotSession: snapshot.session,
      remoteSession,
    })
    await remoteAutomation(remoteSocket, "exit", {}, { runSsh, shellQuote, assert }).catch(() => {})
    await waitForCliExit(remoteCli, "remote CLI")
    await endSessionIfPresent(verificationClient, requests, remoteSession.id)
    log("remote-cli-pass", {
      host: remoteCliHost,
      sessionId: remoteSession.id,
      alias: remoteAlias,
    })
  } finally {
    await terminateChild(remoteCli)
    await runSsh(`rm -f ${shellQuote(remoteSocket)}; rm -rf ${shellQuote(remoteWorkspace)}`).catch(() => {})
  }
}
