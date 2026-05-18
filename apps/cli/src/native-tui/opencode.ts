import { appendFileSync } from "node:fs"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import {
  type RuntimeProviderRun,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  promptAttachmentTransferIsForced,
} from "../prompt-attachment-transfer.js"
import { grantNativeCapabilities } from "./capability-grants.js"
import {
  defaultKernelEndpoint,
  inferWorkspaceTargetsFromLaunchDirectory,
  parseKernelPort,
  parseNativeMode as parseMode,
  parseNativePermissions as parsePermissions,
  reserveLocalPort as reservePort,
} from "./launch-environment.js"
import {
  getNativeProviderRun,
  requestNativeProviderRunLaunch,
} from "./provider-run-control.js"
import {
  type OpenCodeServerProcess,
  runOpenCodeAttach,
  startOpenCodeServer,
} from "./opencode-launcher.js"
import {
  startOpenCodeProxy,
  type OpenCodeNativeProxy,
} from "./opencode-proxy.js"
import { startNativeKernelPumpLoop } from "./native-kernel-pump.js"
import { bridgeRemoteNativeProviderEndpoint } from "./remote-endpoint-bridge.js"
import {
  attachNativeSession,
  createNativeSession,
  prepareCreatedNativeAgent,
  resolveNativeSession,
  spawnNativeAgent,
} from "./session-control.js"

type NativeOpenCodeOptions = {
  sessionRef?: string
  socketPath?: string
  kernelUrl?: string
  kernelPort?: string
  relayUrl?: string
  relayToken?: string
  targetDaemonId?: string
  targetDaemonAlias?: string
  clientId: string
  workspace?: string
  worktree?: string
  machineRef?: string
  sliceRef?: string
  alias?: string
  agentAlias?: string
  mode: "build" | "plan"
  permissions: "required" | "yolo"
  serverInKernel: boolean
  grantMcps: string[]
  grantSkills: string[]
}

export async function runOpenCodeNativeTui(args: string[]): Promise<void> {
  const options = parseNativeOpenCodeArgs(args)
  const inferredTargets = await inferWorkspaceTargetsFromLaunchDirectory(process.cwd())
  const workspace = options.workspace ?? inferredTargets.workspace
  const worktree = options.worktree ?? inferredTargets.worktree
  const kernelEndpoint = options.relayUrl ?? options.kernelUrl ?? options.socketPath ?? defaultKernelEndpoint(options.kernelPort)
  const client = new LocalIpcClient(kernelEndpoint, options.relayUrl
    ? {
      relayAuthToken: options.relayToken,
      targetDaemonId: options.targetDaemonId,
      targetDaemonAlias: options.targetDaemonAlias,
    }
    : undefined)

  let proxy: OpenCodeNativeProxy | null = null
  let openCodeServer: OpenCodeServerProcess | null = null
  let endpointBridge: { close: () => Promise<void> } | null = null
  let pump: { stop: () => void } | null = null
  try {
    const remotePlacement = Boolean(options.machineRef || options.sliceRef)
    const created = options.sessionRef
      ? null
      : await createNativeSession(client, workspace, worktree, options.alias, {
        provider: "opencode",
        model: "default",
        effort: null,
        execution_mode: options.mode,
        permission_level: options.permissions,
      }, options.sliceRef)
    const session = created?.session ?? (options.sessionRef
      ? await resolveNativeSession(client, options.sessionRef, workspace)
      : (() => {
        throw new Error("missing OpenCode session")
      })())
    const attachment = await attachNativeSession(client, session.id, options.clientId)
    const agent = created?.agent
      ? await prepareCreatedNativeAgent(client, session.id, created.agent, options.agentAlias, options.machineRef)
      : await spawnNativeAgent(client, session.id, "opencode", options.agentAlias, "default", undefined, null, options.mode, options.permissions, options.machineRef, options.sliceRef)
    await grantNativeCapabilities(client, workspace, agent.id, options.grantMcps, options.grantSkills)
    let upstreamBaseUrl: string
    let run: RuntimeProviderRun | null = null
    if (options.serverInKernel) {
      const launched = await requestNativeProviderRunLaunch(client, {
        sessionId: session.id,
        provider: "opencode",
        model: "default",
        effort: "",
        agentId: agent.id,
      })
      run = !remotePlacement && launched.session_id === session.id
        ? await waitForOpenCodeRunReady(client, launched.id)
        : launched
      if (!run.structured_endpoint) {
        throw new Error("OpenCode managed native server did not expose an endpoint")
      }
      upstreamBaseUrl = run.structured_endpoint
    } else {
      upstreamBaseUrl = `http://127.0.0.1:${await reservePort()}`
      openCodeServer = await startOpenCodeServer(upstreamBaseUrl, worktree)
    }
    const bridgedEndpoint = await bridgeRemoteNativeProviderEndpoint(upstreamBaseUrl, "OpenCode")
    upstreamBaseUrl = bridgedEndpoint.endpoint
    endpointBridge = bridgedEndpoint
    proxy = await startOpenCodeProxy({
      upstreamBaseUrl,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      inlineLocalAttachments: Boolean(options.relayUrl) || remotePlacement || promptAttachmentTransferIsForced(),
      debug: debugNativeMutation,
    })
    const proxyAddress = proxy.server.address()
    if (!proxyAddress || typeof proxyAddress === "string") {
      throw new Error("OpenCode proxy did not expose a TCP port")
    }
    const proxyUrl = `http://127.0.0.1:${proxyAddress.port}`
    if (!options.serverInKernel) {
      const launched = await requestNativeProviderRunLaunch(client, {
        sessionId: session.id,
        provider: "opencode",
        model: "default",
        effort: "",
        agentId: agent.id,
        native: { structuredEndpoint: proxyUrl },
      })
      run = !remotePlacement && launched.session_id === session.id
        ? await waitForOpenCodeRunReady(client, launched.id)
        : launched
    }
    if (!run) {
      throw new Error("OpenCode provider run was not launched")
    }
    const providerSessionId = run.provider_session_id
    if (!providerSessionId) {
      throw new Error("OpenCode provider run did not expose provider_session_id")
    }
    proxy.bindProviderRun(run)
    process.stderr.write([
      "[arroba opencode native-tui]",
      `  arroba session: ${session.id}${session.alias ? ` (${session.alias})` : ""}`,
      `  arroba agent:   ${agent.id}${agent.alias ? ` (${agent.alias})` : ""}`,
      `  provider run:   ${run.id}`,
      `  opencode sess:  ${providerSessionId}`,
      `  opencode server:${upstreamBaseUrl}`,
      `  proxy:          ${proxyUrl}`,
      "  prompt policy:  native prompts pass through; Arroba observes the session",
      "",
    ].join("\n"))
    pump = startNativeKernelPumpLoop(client, session.id, attachment.id, {
      debug: debugNativeMutation,
      formatError,
    })

    await runOpenCodeAttach({
      proxyUrl,
      providerSessionId,
      workingDirectory: worktree,
    })
  } finally {
    pump?.stop()
    if (proxy) {
      await new Promise<void>((resolve) => proxy!.server.close(() => resolve()))
    }
    if (openCodeServer && openCodeServer.exitCode == null) {
      openCodeServer.kill("SIGTERM")
      await Promise.race([
        new Promise((resolve) => openCodeServer?.once("exit", resolve)),
        sleep(2_000),
      ])
      if (openCodeServer.exitCode == null) openCodeServer.kill("SIGKILL")
    }
    await endpointBridge?.close()
    await client.close()
  }
}

function parseNativeOpenCodeArgs(args: string[]): NativeOpenCodeOptions {
  const options: NativeOpenCodeOptions = {
    clientId: `arroba-opencode-native-${process.pid}`,
    mode: "build",
    permissions: "yolo",
    serverInKernel: false,
    grantMcps: [],
    grantSkills: [],
  }
  const positional: string[] = []

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index] ?? ""
    const next = () => {
      const value = args[index + 1]
      if (!value) {
        throw new Error(`missing value for ${arg}`)
      }
      index += 1
      return value
    }

    switch (arg) {
      case "--socket":
        options.socketPath = next()
        break
      case "--kernel-url":
        options.kernelUrl = next()
        break
      case "--kernel-port":
      case "--port":
        options.kernelPort = parseKernelPort(next(), arg)
        break
      case "--relay-url":
        options.relayUrl = next()
        break
      case "--relay-token":
        options.relayToken = next()
        break
      case "--target-daemon-id":
        options.targetDaemonId = next()
        break
      case "--target-daemon-alias":
        options.targetDaemonAlias = next()
        break
      case "--client-id":
        options.clientId = next()
        break
      case "--workspace":
        options.workspace = path.resolve(next())
        break
      case "--worktree":
        options.worktree = path.resolve(next())
        break
      case "--machine":
      case "--kernel-ref":
        options.machineRef = next()
        break
      case "--slice":
        options.sliceRef = next()
        break
      case "--alias":
        options.alias = next()
        break
      case "--agent-alias":
        options.agentAlias = next()
        break
      case "--mode":
        options.mode = parseMode(next())
        break
      case "--permissions":
        options.permissions = parsePermissions(next())
        break
      case "--server-in-kernel":
        options.serverInKernel = true
        break
      case "--grant-mcp":
        options.grantMcps.push(next())
        break
      case "--grant-skill":
        options.grantSkills.push(next())
        break
      case "--help":
      case "-h":
        printNativeOpenCodeUsage()
        process.exit(0)
      default:
        if (arg.startsWith("-")) {
          throw new Error(`unknown opencode argument ${arg}`)
        }
        positional.push(arg)
    }
  }

  if (positional.length > 1) {
    throw new Error("usage: arroba opencode [session-ref]")
  }
  if (options.relayUrl && !options.relayToken) {
    throw new Error("--relay-url requires --relay-token")
  }
  if (options.relayUrl && !options.targetDaemonId && !options.targetDaemonAlias) {
    throw new Error("--relay-url requires --target-daemon-id or --target-daemon-alias")
  }
  if (options.relayUrl && (options.kernelUrl || options.socketPath)) {
    throw new Error("--relay-url cannot be used together with --kernel-url or --socket")
  }
  if (options.relayUrl && options.kernelPort) {
    throw new Error("--relay-url cannot be used together with --kernel-port")
  }
  if (options.kernelUrl && options.kernelPort) {
    throw new Error("--kernel-url cannot be used together with --kernel-port")
  }
  if (options.socketPath && options.kernelPort) {
    throw new Error("--socket cannot be used together with --kernel-port")
  }
  if (options.machineRef && options.sliceRef) {
    throw new Error("--machine and --slice cannot be used together")
  }
  if (options.machineRef && !options.serverInKernel) {
    throw new Error("--machine requires --server-in-kernel so the OpenCode server is launched by the worker kernel")
  }
  if (options.sliceRef && !options.serverInKernel) {
    throw new Error("--slice requires --server-in-kernel so the OpenCode server is launched by the slice worker kernel")
  }
  if (positional[0] !== undefined) {
    options.sessionRef = positional[0]
  }
  return options
}

function printNativeOpenCodeUsage() {
  process.stdout.write([
    "usage: arroba opencode [session-ref] [--socket PATH|--kernel-url URL|--kernel-port PORT] [--mode build|plan] [--permissions required|yolo]",
    "       arroba opencode [session-ref] --relay-url URL --relay-token TOKEN (--target-daemon-id ID|--target-daemon-alias NAME)",
    "",
    "placement:",
    "  --machine, --kernel-ref REF       Run the Arroba agent/provider on a remote worker kernel",
    "  --slice REF                       Run the Arroba agent/provider on a home-managed slice worker",
    "",
    "behavior:",
    "  --grant-mcp NAME                Grant an installed Arroba MCP to the native agent before provider launch",
    "  --grant-skill NAME              Grant an installed Arroba skill to the native agent before provider launch",
    "  creates a new Arroba agent in the selected session and launches native `opencode attach` for it.",
  ].join("\n") + "\n")
}

async function waitForOpenCodeRunReady(
  client: LocalIpcClient,
  providerRunId: string,
): Promise<RuntimeProviderRun> {
  const deadline = Date.now() + 60_000
  let latest: RuntimeProviderRun | null = null
  while (Date.now() < deadline) {
    latest = await getNativeProviderRun(client, providerRunId)
    if (
      latest.state === "Running"
      && latest.structured_endpoint
      && latest.provider_session_id
    ) {
      return latest
    }
    if (latest.state === "Ended") {
      throw new Error(`OpenCode provider run ended before attach was ready: ${providerRunId}`)
    }
    await sleep(250)
  }
  throw new Error(`timed out waiting for OpenCode provider run to become ready: ${providerRunId} (${latest?.state ?? "unknown"})`)
}

function formatError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function debugNativeMutation(label: string, payload: unknown) {
  if (!process.env.ARROBA_OPENCODE_NATIVE_DEBUG) return
  const line = `[arroba opencode native-tui] ${label}: ${JSON.stringify(payload)}\n`
  const debugFile = process.env.ARROBA_OPENCODE_NATIVE_DEBUG_FILE
  if (debugFile) {
    appendFileSync(debugFile, line)
    return
  }
  process.stderr.write(line)
}
