#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'
import { assertBinary, terminateChild, waitForTcpPort } from './lib/drill-runtime-helpers.mjs'
import { joinRemoteHomeExtensionCollaborator } from './lib/remote-home-extension-collab-helpers.mjs'
import { createHomeExtensionFixtures } from './lib/home-extension-fixtures.mjs'
import {
  ensureRemoteHomeExtensionHetznerWorkspace,
  removeRemoteHomeExtensionHetznerRoot,
  spawnRemoteHomeExtensionHetznerWorker,
  startRemoteHomeExtensionHetznerRelay,
  stopRemoteHomeExtensionHetznerRelay,
  stopRemoteHomeExtensionHetznerWorker,
} from './lib/remote-home-extension-hetzner-helpers.mjs'
import { createRemoteHomeExtensionRelayTokenFactory } from './lib/remote-home-extension-relay-helpers.mjs'
import { waitForDaemon, waitForRelayTarget, waitForRemoteMachine } from './lib/remote-home-extension-session-helpers.mjs'
import { callRuntimeMcp, expectReject, expectRuntimeMcpReject, waitForRuntimeTool } from './lib/runtime-mcp-assertions.mjs'
import { runHetznerCommand, shellQuote } from './lib/native-tui-remote-execution.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const realHomeDir = os.homedir()
const RELAY_ISSUER = 'arroba-remote-home-extension-drill'
const RELAY_SECRET = 'arroba-remote-home-extension-drill-secret'
const RELAY_REALM = 'remote-home-extension-drill'

function parseArgs(argv) {
  const options = {
    hetznerWorker: false,
    collab: false,
    hetznerHost: process.env.ARROBA_REMOTE_HOME_EXTENSION_HETZNER_HOST ?? process.env.ARROBA_NATIVE_TUI_HETZNER_HOST ?? 'root@195.201.123.115',
    hetznerKey: process.env.ARROBA_REMOTE_HOME_EXTENSION_HETZNER_KEY ?? process.env.ARROBA_NATIVE_TUI_HETZNER_KEY ?? path.join(os.homedir(), '.ssh', 'arroba_hetzner_staging'),
    hetznerRepo: process.env.ARROBA_REMOTE_HOME_EXTENSION_HETZNER_REPO ?? process.env.ARROBA_NATIVE_TUI_HETZNER_REPO ?? '/tmp/arroba-native-remote-validate',
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--hetzner-worker') options.hetznerWorker = true
    else if (arg === '--collab') options.collab = true
    else if (arg === '--hetzner-host') options.hetznerHost = argv[++i]
    else if (arg === '--hetzner-key') options.hetznerKey = argv[++i]
    else if (arg === '--hetzner-repo') options.hetznerRepo = argv[++i]
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-remote-home-extension-drill.mjs [--hetzner-worker]',
    '',
    'Runs a home-owned remote extension drill with home/worker kernels:',
    '- projects home-owned script, MCP, and connector tools to a remote agent',
    '- verifies execution remains on home',
    '- verifies revoke stops worker advertisement and stale calls',
    '- verifies worker-local MCP name collision fails before launch',
    '- with --collab, verifies a collaborator-owned remote agent can invoke home grants but cannot grant/revoke/request them',
    '',
    '  --hetzner-worker       Run relay and worker kernel on the configured Hetzner host',
    '  --collab              Run the main agent path as user-2 over a scoped relay',
    '  --hetzner-host HOST    SSH host for --hetzner-worker',
    '  --hetzner-key PATH     SSH key for --hetzner-worker',
    '  --hetzner-repo PATH    Remote Arroba checkout containing built binaries',
  ].join('\n'))
}

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const {
  attachToSessionRequest,
  createSessionInviteRequest,
  createSessionRequest,
  grantAgentExtensionRequest,
  joinSessionInviteRequest,
  launchProviderRunRequest,
  listRemoteMachinesRequest,
  registerConnectorAdapterRequest,
  registerConnectorRequest,
  registerEnvironmentRequest,
  registerScriptRequest,
  revokeAgentExtensionRequest,
  spawnAgentRequest,
} = await import('../../../packages/kernel-client/dist/ipc-requests.js')

const unwrap = (response, key) => response?.[key] ?? response
const unwrapVariant = (response, ...keys) => keys.map((key) => response?.[key]).find((value) => value != null) ?? response

function log(name, details = null) {
  if (details == null) console.log(`[remote-home-extension-drill] ${name}`)
  else console.log(`[remote-home-extension-drill] ${name}`, JSON.stringify(details))
}

async function resolveBinary(binaryPath, manifestPath, binName) {
  await assertBinary(binaryPath, manifestPath, binName)
  return binaryPath
}

async function resolveExecutable(command) {
  if (command.includes(path.sep)) {
    await access(command)
    return command
  }
  for (const dir of (process.env.PATH ?? '').split(path.delimiter)) {
    if (!dir) continue
    const candidate = path.join(dir, command)
    try {
      await access(candidate)
      return candidate
    } catch {}
  }
  throw new Error(`executable ${command} not found on PATH`)
}

function daemonEnv({ rootDir, relayUrl, relayToken, daemonId, daemonAlias, machineId, machineAlias, kernelPort, mcpPort, acceptRemoteLeases, capabilityRoot, socketName }) {
  return {
    ...process.env,
    HOME: path.join(rootDir, `${daemonAlias}-home`),
    CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, '.codex'),
    OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(realHomeDir, '.config', 'opencode'),
    XDG_CONFIG_HOME: path.join(rootDir, `${daemonAlias}-config`),
    XDG_STATE_HOME: path.join(rootDir, `${daemonAlias}-state`),
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
    ARROBA_CODEX_PORT: String(kernelPort + 2001),
    ARROBA_RELAY_URL: relayUrl,
    ARROBA_RELAY_TOKEN: relayToken,
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_MACHINE_ID: machineId,
    ARROBA_MACHINE_ALIAS: machineAlias,
    ARROBA_ACCEPT_REMOTE_LEASES: acceptRemoteLeases ? '1' : '0',
    ARROBA_DAEMON_SOCKET: path.join(rootDir, socketName),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, `${daemonAlias}-history`),
    ARROBA_CAPABILITY_ISOLATION_ROOT: capabilityRoot,
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const python = await resolveExecutable(process.env.PYTHON ?? 'python3')
  const rootDir = path.join(os.tmpdir(), `arroba-remote-home-extension-${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const homeMarker = path.join(rootDir, 'home-script-marker.txt')
  const homeMcpMarker = path.join(rootDir, 'home-mcp-marker.txt')
  const homeConnectorMarker = path.join(rootDir, 'home-connector-marker.txt')
  const portsBase = 58100 + Math.floor(Math.random() * 500)
  const relayPort = portsBase
  const homeOnlyMcpPort = portsBase + 500
  const homeKernelPort = portsBase + 1000
  const workerKernelPort = portsBase + 1001
  const homeMcpPort = portsBase + 2000
  const workerMcpPort = portsBase + 2001
  const sharedRelayToken = `remote-home-extension-${process.pid}`
  const homeDaemonId = `remote-home-extension-home-${process.pid}`
  const workerDaemonId = `remote-home-extension-worker-${process.pid}`
  const { daemonToken, clientToken } = createRemoteHomeExtensionRelayTokenFactory({
    issuer: RELAY_ISSUER,
    secret: RELAY_SECRET,
    realm: RELAY_REALM,
  })
  const homeRelayToken = options.collab ? daemonToken(homeDaemonId, 'home-user') : sharedRelayToken
  const workerRelayToken = options.collab ? daemonToken(workerDaemonId, 'worker-user') : sharedRelayToken
  const probeRelayToken = options.collab ? clientToken('home-user') : sharedRelayToken
  const relayUrl = `ws://127.0.0.1:${relayPort}`
  const workerMachineId = `remote-home-extension-worker-machine-${process.pid}`
  const workerAlias = `remote-home-extension-worker-${process.pid}`
  const workerKernelRef = options.collab ? workerDaemonId : workerAlias
  const remoteRoot = `/tmp/arroba-remote-home-extension-${process.pid}-${Date.now()}`
  const workerWorktree = options.hetznerWorker ? path.posix.join(remoteRoot, 'workspace') : workspace

  let relay = null
  let relayTunnel = null
  let home = null
  let worker = null
  let homeOnlyMcp = null
  let client = null
  let user2Client = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    const fixtures = await createHomeExtensionFixtures({
      rootDir,
      workspace,
      homeOnlyMcpPort,
      homeMarker,
      homeMcpMarker,
      homeConnectorMarker,
    })
    const {
      connectorAdapterPath,
      connectorPath,
      homeCapabilityRoot,
      scriptPath,
      workerCapabilityRoot,
    } = fixtures
    homeOnlyMcp = fixtures.homeOnlyMcp

    const relayBinary = options.hetznerWorker
      ? path.posix.join(options.hetznerRepo, 'apps/relay/target/debug/arroba-relay')
      : await resolveBinary(path.join(repoRoot, 'apps/relay/target/debug/arroba-relay'), path.join(repoRoot, 'apps/relay/Cargo.toml'), 'arroba-relay')
    const daemonBinary = await resolveBinary(path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'), path.join(repoRoot, 'apps/kernel/Cargo.toml'), 'arroba-kernel')
    if (options.hetznerWorker) {
      await ensureRemoteHomeExtensionHetznerWorkspace(options, { remoteRoot, workerWorktree })
      const hetznerRelay = await startRemoteHomeExtensionHetznerRelay({
        options,
        relayPort,
        workerMcpPort,
        sharedRelayToken,
        collab: options.collab,
        issuer: RELAY_ISSUER,
        secret: RELAY_SECRET,
        remoteRoot,
      })
      relay = hetznerRelay.relay
      relayTunnel = hetznerRelay.tunnel
    } else {
      relay = spawn(relayBinary, [], {
        cwd: repoRoot,
        env: {
          ...process.env,
          ARROBA_RELAY_HOST: '127.0.0.1',
          ARROBA_RELAY_PORT: String(relayPort),
          ARROBA_RELAY_TOKEN: sharedRelayToken,
          ...(options.collab ? {
            ARROBA_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
            ARROBA_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
          } : {}),
        },
        stdio: ['ignore', 'ignore', 'inherit'],
      })
      await waitForTcpPort(relayPort)
    }

    home = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        rootDir,
        relayUrl,
        relayToken: homeRelayToken,
        daemonId: homeDaemonId,
        daemonAlias: 'home',
        machineId: `remote-home-extension-home-machine-${process.pid}`,
        machineAlias: `remote-home-extension-home-machine-${process.pid}`,
        kernelPort: homeKernelPort,
        mcpPort: homeMcpPort,
        acceptRemoteLeases: false,
        capabilityRoot: homeCapabilityRoot,
        socketName: 'home.sock',
      }),
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    if (options.hetznerWorker) {
      worker = spawnRemoteHomeExtensionHetznerWorker({
        options,
        remoteRoot,
        workerWorktree,
        relayPort,
        workerRelayToken,
        workerDaemonId,
        workerMachineId,
        workerAlias,
        workerKernelPort,
        workerMcpPort,
      })
    } else {
      worker = spawn(daemonBinary, [], {
        cwd: repoRoot,
        env: daemonEnv({
          rootDir,
          relayUrl,
          relayToken: workerRelayToken,
          daemonId: workerDaemonId,
          daemonAlias: 'worker',
          machineId: workerMachineId,
          machineAlias: workerAlias,
          kernelPort: workerKernelPort,
          mcpPort: workerMcpPort,
          acceptRemoteLeases: true,
          capabilityRoot: workerCapabilityRoot,
          socketName: 'worker.sock',
        }),
        stdio: ['ignore', 'ignore', 'inherit'],
      })
    }
    const homeUrl = `ws://127.0.0.1:${homeKernelPort}`
    await waitForDaemon(LocalIpcClient, homeUrl, listRemoteMachinesRequest)
    if (!options.hetznerWorker) {
      await waitForDaemon(LocalIpcClient, `ws://127.0.0.1:${workerKernelPort}`, listRemoteMachinesRequest)
    }
    await waitForRelayTarget(LocalIpcClient, relayUrl, probeRelayToken, 'home', listRemoteMachinesRequest)
    await waitForRelayTarget(LocalIpcClient, relayUrl, probeRelayToken, 'worker', listRemoteMachinesRequest)
    client = new LocalIpcClient(homeUrl)
    if (!options.collab) {
      await waitForRemoteMachine(client, workerMachineId, listRemoteMachinesRequest)
    }

    const env = unwrap(await client.send(registerEnvironmentRequest(workspace, {
      name: 'home-python',
      runtime: { type: 'python', python },
    })), 'EnvironmentRegistered').environment
    await client.send(registerScriptRequest(workspace, scriptPath, env.name, 'home_only_lookup'))
    await client.send(registerConnectorAdapterRequest(connectorAdapterPath))
    await client.send(registerConnectorRequest(connectorPath))
    const session = unwrap(await client.send(createSessionRequest(workspace, workspace, 'remote-home-extension-drill')), 'SessionCreated').session
    await client.send(attachToSessionRequest(session.id, `remote-home-extension-${process.pid}`))
    if (options.collab) {
      user2Client = await joinRemoteHomeExtensionCollaborator({
        LocalIpcClient,
        client,
        relayUrl,
        clientToken,
        createSessionInviteRequest,
        joinSessionInviteRequest,
        sessionId: session.id,
      })
    }
    const remoteAgentClient = options.collab ? user2Client : client
    if (!options.collab) {
      const workerMcpDir = path.join(workerCapabilityRoot, 'user', 'mcps')
      const workerCollisionMcp = `${JSON.stringify({
        name: 'home_echo_mcp',
        transport: {
          type: 'streamable_http',
          url: `http://127.0.0.1:${homeOnlyMcpPort}/mcp`,
        },
        enabled: true,
        required: false,
      }, null, 2)}\n`
      if (options.hetznerWorker) {
        const remoteWorkerMcpDir = path.posix.join(remoteRoot, 'worker-capabilities', 'user', 'mcps')
        const remoteWorkerMcpPath = path.posix.join(remoteWorkerMcpDir, 'home_echo_mcp.json')
        await runHetznerCommand(options, `mkdir -p ${shellQuote(remoteWorkerMcpDir)} && cat > ${shellQuote(remoteWorkerMcpPath)} <<'EOF'\n${workerCollisionMcp}EOF`)
      } else {
        await mkdir(workerMcpDir, { recursive: true })
        await writeFile(path.join(workerMcpDir, 'home_echo_mcp.json'), workerCollisionMcp, 'utf8')
      }
      const collisionAgent = unwrap(await client.send(spawnAgentRequest(session.id, 'dev-stub', 'home-proxy-collision-agent', 'default', workerWorktree, 'low', undefined, undefined, workerKernelRef)), 'AgentSpawned').agent
      await client.send(grantAgentExtensionRequest(workspace, collisionAgent.id, 'mcp', 'home_echo_mcp'))
      await expectReject('home-proxy MCP collision launch', () => client.send(launchProviderRunRequest(
        session.id,
        'dev-stub',
        'default',
        'default',
        'low',
        collisionAgent.id,
        { nativeTui: true },
      )), 'collides with a worker-local MCP')
      if (options.hetznerWorker) {
        await runHetznerCommand(options, `rm -f ${shellQuote(path.posix.join(remoteRoot, 'worker-capabilities', 'user', 'mcps', 'home_echo_mcp.json'))}`)
      } else {
        await rm(path.join(workerMcpDir, 'home_echo_mcp.json'), { force: true })
      }
    }

    const agent = unwrap(await remoteAgentClient.send(spawnAgentRequest(session.id, 'dev-stub', 'home-proxy-agent', 'default', workerWorktree, 'low', undefined, undefined, workerKernelRef)), 'AgentSpawned').agent
    if (options.collab) {
      await expectReject(
        'collaborator grant home script',
        () => user2Client.send(grantAgentExtensionRequest(workspace, agent.id, 'script', 'home_only_lookup', env.name)),
        'home extensions for remote-backed agent',
      )
    }
    await client.send(grantAgentExtensionRequest(workspace, agent.id, 'script', 'home_only_lookup', env.name))
    await client.send(grantAgentExtensionRequest(workspace, agent.id, 'mcp', 'home_echo_mcp'))
    await client.send(grantAgentExtensionRequest(workspace, agent.id, 'connector', 'home_local_api', null, { maxSafety: 'read' }))
    const launch = unwrapVariant(await remoteAgentClient.send(launchProviderRunRequest(
      session.id,
      'dev-stub',
      'default',
      'default',
      'low',
      agent.id,
      { nativeTui: true },
    )), 'ProviderRunLaunched', 'ProviderRunLaunchAccepted').provider_run
    if (!launch.runtime_mcp_server_url || !launch.runtime_mcp_auth_token) throw new Error(`launched run lacks runtime MCP binding: ${JSON.stringify(launch)}`)
    if (options.collab) {
      const deniedRequest = await callRuntimeMcp(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'tools/call', {
        name: 'arroba.request_extension',
        arguments: { kind: 'script', name: 'home_only_lookup', environment: env.name },
      })
      if (!deniedRequest.isError) {
        const deniedText = JSON.stringify(deniedRequest)
        if (!deniedText.includes('home-owned extensions for collaborator remote agents')) {
          throw new Error(`collaborator runtime request_extension returned unexpected result: ${deniedText}`)
        }
      }
    }

    await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'home_only_lookup', true)
    const call = await callRuntimeMcp(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_only_lookup',
      arguments: { query: 'remote-agent' },
    })
    if (call.isError) throw new Error(`home proxy script returned error: ${JSON.stringify(call)}`)
    const marker = await readFile(homeMarker, 'utf8')
    if (marker !== 'HOME_SCRIPT_EXECUTED:remote-agent') throw new Error(`home marker mismatch: ${JSON.stringify(marker)}`)
    const proxyUrl = launch.runtime_mcp_server_url.replace(/\/mcp\/?$/, '/mcp/proxy/home_echo_mcp')
    const mcpTools = await callRuntimeMcp(proxyUrl, launch.runtime_mcp_auth_token, 'tools/list')
    if (!mcpTools.tools.some((tool) => tool.name === 'home_echo')) throw new Error(`home MCP tool not listed: ${JSON.stringify(mcpTools)}`)
    const mcpCall = await callRuntimeMcp(proxyUrl, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_echo',
      arguments: { text: 'remote-mcp' },
    })
    if (mcpCall.isError) throw new Error(`home MCP returned error: ${JSON.stringify(mcpCall)}`)
    const mcpMarker = await readFile(homeMcpMarker, 'utf8')
    if (mcpMarker !== 'HOME_MCP_EXECUTED:remote-mcp') throw new Error(`home MCP marker mismatch: ${JSON.stringify(mcpMarker)}`)
    await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'home_local_api_public_echo', true)
    const connectorCall = await callRuntimeMcp(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_local_api_public_echo',
      arguments: { q: 'remote-connector' },
    })
    if (connectorCall.isError) throw new Error(`home connector returned error: ${JSON.stringify(connectorCall)}`)
    const connectorMarker = await readFile(homeConnectorMarker, 'utf8')
    if (connectorMarker !== 'HOME_CONNECTOR_EXECUTED:remote-connector') throw new Error(`home connector marker mismatch: ${JSON.stringify(connectorMarker)}`)

    await client.send(revokeAgentExtensionRequest(agent.id, 'script', 'home_only_lookup'))
    if (options.collab) {
      await expectReject(
        'collaborator revoke home MCP',
        () => user2Client.send(revokeAgentExtensionRequest(agent.id, 'mcp', 'home_echo_mcp')),
        'home extensions for remote-backed agent',
      )
    }
    await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'home_only_lookup', false)
    await expectRuntimeMcpReject(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_only_lookup',
      arguments: { query: 'after-revoke' },
    })
    const afterRevokeMarker = await readFile(homeMarker, 'utf8')
    if (afterRevokeMarker !== marker) throw new Error('revoked home-proxy script executed after revoke')
    await client.send(revokeAgentExtensionRequest(agent.id, 'mcp', 'home_echo_mcp'))
    await expectRuntimeMcpReject(proxyUrl, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_echo',
      arguments: { text: 'after-mcp-revoke' },
    })
    const afterMcpRevokeMarker = await readFile(homeMcpMarker, 'utf8')
    if (afterMcpRevokeMarker !== mcpMarker) throw new Error('revoked home-proxy MCP executed after revoke')
    await client.send(revokeAgentExtensionRequest(agent.id, 'connector', 'home_local_api'))
    await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'home_local_api_public_echo', false)
    await expectRuntimeMcpReject(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_local_api_public_echo',
      arguments: { q: 'after-connector-revoke' },
    })
    const afterConnectorRevokeMarker = await readFile(homeConnectorMarker, 'utf8')
    if (afterConnectorRevokeMarker !== connectorMarker) throw new Error('revoked home-proxy connector executed after revoke')

    succeeded = true
    console.log('[remote-home-extension-drill] pass', JSON.stringify({ mode: options.hetznerWorker ? 'hetzner-worker' : 'local-worker', collab: options.collab, script: 'home_only_lookup', mcp: 'home_echo_mcp', connector: 'home_local_api', workerAlias, revoke: true }))
  } catch (error) {
    failure = error
    throw error
  } finally {
    await user2Client?.close?.().catch(() => {})
    await client?.close?.().catch(() => {})
    await terminateChild(worker)
    await terminateChild(home)
    await terminateChild(relayTunnel)
    await terminateChild(relay)
    if (options.hetznerWorker) {
      await stopRemoteHomeExtensionHetznerWorker(options, { remoteRoot, workerDaemonId })
      await stopRemoteHomeExtensionHetznerRelay(options, remoteRoot)
      await removeRemoteHomeExtensionHetznerRoot(options, remoteRoot)
    }
    await new Promise((resolve) => homeOnlyMcp?.close?.(resolve) ?? resolve())
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      failure,
      metadata: {
        drill: 'remote-home-extension',
        mode: options.hetznerWorker ? 'hetzner-worker' : 'local-worker',
        collab: options.collab,
      },
      log,
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
