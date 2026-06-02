import { spawn } from "node:child_process"
import { appendFileSync } from "node:fs"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import {
  type RuntimeProviderRun,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import { promptAttachmentTransferIsForced } from "../prompt-attachment-transfer.js"
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
  type CodexAppServerProcess,
  releaseKernelPortLocks,
  reserveCodexKernelServerPort,
  startCodexAppServer,
  startCodexAppServerInKernel,
  stopCodexAppServerInKernel,
} from "./codex-app-server.js"
import {
  startCodexProxy,
  type CodexProxyServer,
} from "./codex-proxy.js"
import {
  type CodexNativeBindingState,
} from "./codex-turn-submission.js"
import { startNativeKernelPumpLoop } from "./native-kernel-pump.js"
import {
  requestNativeProviderRunLaunch,
} from "./provider-run-control.js"
import { bridgeRemoteNativeProviderEndpoint } from "./remote-endpoint-bridge.js"
import {
  formatNativeTuiRuntimeBanner,
} from "./runtime-banner.js"
import { loadNativeTuiSliceInventory } from "./slice-inventory.js"
import {
  attachNativeSession,
  createNativeSession,
  prepareCreatedNativeAgent,
  resolveNativeSession,
  spawnNativeAgent,
} from "./session-control.js"

type NativeCodexOptions = {
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
  model: string
  effort: string
  mode: "build" | "plan"
  permissions: "required" | "yolo"
  initialPrompt?: string
  serverInKernel: boolean
  grantMcps: string[]
  grantSkills: string[]
}

export async function runCodexNativeTui(args: string[]): Promise<void> {
  const options = parseNativeCodexArgs(args)
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
  let appServer: CodexAppServerProcess | null = null
  let kernelServerPid: string | null = null
  let proxy: CodexProxyServer | null = null
  let endpointBridge: { close: () => Promise<void> } | null = null
  let pump: { stop: () => void } | null = null
  let cleanupSessionId: string | null = null
  let cleanupAttachmentId: string | null = null

  try {
    const remotePlacement = Boolean(options.machineRef || options.sliceRef)
    const created = options.sessionRef
      ? null
      : await createNativeSession(client, workspace, worktree, options.alias, {
        provider: "codex",
        model: options.model,
        effort: options.effort,
        execution_mode: options.mode,
        permission_level: options.permissions,
      }, options.sliceRef)
    const session = created?.session ?? await resolveNativeSession(client, options.sessionRef!, workspace)
    const attachment = await attachNativeSession(client, session.id, options.clientId)
    cleanupSessionId = session.id
    cleanupAttachmentId = attachment.id
    const agent = created?.agent
      ? await prepareCreatedNativeAgent(client, session.id, created.agent, options.agentAlias, options.machineRef)
      : await spawnNativeAgent(client, session.id, "codex", options.agentAlias, options.model, worktree, options.effort, options.mode, options.permissions, options.machineRef, options.sliceRef)
    await grantNativeCapabilities(client, workspace, agent.id, options.grantMcps, options.grantSkills)
    const bindState: CodexNativeBindingState = {
      promise: null,
      run: null,
      structuredEndpoint: null,
    }
    let providerSessionId: string | null = null
    let upstreamEndpoint: string
    let bindProviderEndpoint: string
    if (options.serverInKernel && remotePlacement) {
      const run = await launchManagedNativeProviderRun({
        client,
        sessionId: session.id,
        agentId: agent.id,
        model: options.model,
        effort: options.effort,
      })
      if (!run.structured_endpoint) {
        throw new Error("Codex managed native server did not expose an endpoint")
      }
      bindState.promise = Promise.resolve(run)
      bindState.run = run
      if (run.provider_session_id) {
        providerSessionId = run.provider_session_id
        debugNativeCodex("thread_observed", { threadId: run.provider_session_id })
      }
      upstreamEndpoint = run.structured_endpoint
      bindProviderEndpoint = ""
    } else if (options.serverInKernel) {
      const port = await reserveCodexKernelServerPort()
      upstreamEndpoint = `ws://127.0.0.1:${port}`
      const listenHost = process.env.ARROBA_CODEX_KERNEL_SERVER_BIND_HOST?.trim() || "127.0.0.1"
      const listenEndpoint = `ws://${listenHost}:${port}`
      kernelServerPid = await startCodexAppServerInKernel({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        endpoint: upstreamEndpoint,
        listenEndpoint,
        workingDirectory: worktree,
      })
      bindProviderEndpoint = process.env.ARROBA_CODEX_KERNEL_SERVER_PORT_RANGE ? upstreamEndpoint : ""
    } else {
      upstreamEndpoint = `ws://127.0.0.1:${await reservePort()}`
      appServer = await startCodexAppServer(upstreamEndpoint, worktree)
      bindProviderEndpoint = ""
    }
    const bridgedEndpoint = await bridgeRemoteNativeProviderEndpoint(upstreamEndpoint, "Codex")
    upstreamEndpoint = bridgedEndpoint.endpoint
    endpointBridge = bridgedEndpoint
    proxy = await startCodexProxy({
      upstreamEndpoint,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      model: options.model,
      effort: options.effort,
      bindState,
      inlineLocalAttachments: Boolean(options.relayUrl) || remotePlacement || promptAttachmentTransferIsForced(),
      debug: debugNativeCodex,
    })
    const proxyAddress = proxy.address()
    if (!proxyAddress || typeof proxyAddress === "string") {
      throw new Error("Codex proxy did not expose a TCP port")
    }
    const proxyUrl = `ws://127.0.0.1:${proxyAddress.port}`
    bindState.structuredEndpoint = bindProviderEndpoint || proxyUrl
    const sliceInventory = agent.remote_execution
      ? await loadNativeTuiSliceInventory(client)
      : { slices: [], error: null }
    process.stderr.write(formatNativeTuiRuntimeBanner({
      surface: "codex native-tui",
      session,
      agent,
      worktree,
      slices: sliceInventory.slices,
      sliceLookupError: options.sliceRef ? sliceInventory.error : null,
      run: bindState.run,
      grantedMcps: options.grantMcps,
      grantedSkills: options.grantSkills,
      providerLines: [
        `  app-server:     ${upstreamEndpoint}`,
        `  proxy:          ${proxyUrl}`,
        ...(providerSessionId ? [`  codex thread:   ${providerSessionId}`] : []),
      ],
      promptPolicy: "native prompts pass through; Arroba observes the session",
    }))
    pump = startNativeKernelPumpLoop(client, session.id, attachment.id, {
      onTerminalRecords: remotePlacement
        ? (records) => proxy?.projectKernelOutputToTui(records)
        : undefined,
      debug: debugNativeCodex,
      formatError,
    })
    await runCodexTui({
      proxyUrl,
      model: options.model,
      workingDirectory: worktree,
      providerSessionId: remotePlacement ? null : providerSessionId,
      initialPrompt: options.initialPrompt,
    })
  } finally {
    pump?.stop()
    if (proxy) {
      await new Promise<void>((resolve) => proxy!.close(() => resolve()))
    }
    if (appServer && appServer.exitCode == null) {
      appServer.kill("SIGTERM")
      await Promise.race([
        new Promise((resolve) => appServer?.once("exit", resolve)),
        sleep(2_000),
      ])
      if (appServer.exitCode == null) appServer.kill("SIGKILL")
    }
    if (kernelServerPid) {
      await stopCodexAppServerInKernel(client, cleanupSessionId, cleanupAttachmentId, kernelServerPid, worktree).catch(() => {})
    }
    await endpointBridge?.close()
    releaseKernelPortLocks()
    await client.close()
  }
}

function parseNativeCodexArgs(args: string[]): NativeCodexOptions {
  const options: NativeCodexOptions = {
    clientId: `arroba-codex-native-${process.pid}`,
    model: "gpt-5.4-mini",
    effort: "high",
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
      if (!value) throw new Error(`missing value for ${arg}`)
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
      case "--model":
      case "-m":
        options.model = next()
        break
      case "--effort":
        options.effort = next()
        break
      case "--mode": {
        const value = parseMode(next())
        options.mode = value
        break
      }
      case "--permissions": {
        const value = parsePermissions(next())
        options.permissions = value
        break
      }
      case "--initial-prompt":
        options.initialPrompt = next()
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
        printNativeCodexUsage()
        process.exit(0)
      default:
        if (arg.startsWith("-")) throw new Error(`unknown codex argument ${arg}`)
        positional.push(arg)
    }
  }
  if (positional.length > 1) throw new Error("usage: arroba codex [session-ref]")
  if (options.relayUrl && !options.relayToken) throw new Error("--relay-url requires --relay-token")
  if (options.relayUrl && !options.targetDaemonId && !options.targetDaemonAlias) {
    throw new Error("--relay-url requires --target-daemon-id or --target-daemon-alias")
  }
  if (options.relayUrl && (options.kernelUrl || options.socketPath)) {
    throw new Error("--relay-url cannot be used together with --kernel-url or --socket")
  }
  if (options.relayUrl && options.kernelPort) throw new Error("--relay-url cannot be used together with --kernel-port")
  if (options.kernelUrl && options.kernelPort) throw new Error("--kernel-url cannot be used together with --kernel-port")
  if (options.socketPath && options.kernelPort) throw new Error("--socket cannot be used together with --kernel-port")
  if (options.machineRef && options.sliceRef) {
    throw new Error("--machine and --slice cannot be used together")
  }
  if (options.machineRef && !options.serverInKernel) {
    throw new Error("--machine requires --server-in-kernel so the Codex app-server is launched by the worker kernel")
  }
  if (options.sliceRef && !options.serverInKernel) {
    throw new Error("--slice requires --server-in-kernel so the Codex app-server is launched by the slice worker kernel")
  }
  if (positional[0] !== undefined) options.sessionRef = positional[0]
  return options
}

function printNativeCodexUsage() {
  process.stdout.write([
    "usage: arroba codex [session-ref] [--socket PATH|--kernel-url URL|--kernel-port PORT] [--mode build|plan] [--permissions required|yolo]",
    "       arroba codex [session-ref] --relay-url URL --relay-token TOKEN (--target-daemon-id ID|--target-daemon-alias NAME)",
    "",
    "placement:",
    "  --machine, --kernel-ref REF       Run the Arroba agent/provider on a remote worker kernel",
    "  --slice REF                       Run the Arroba agent/provider on a home-managed slice worker",
    "",
    "behavior:",
    "  --grant-mcp NAME                Grant an installed Arroba MCP to the native agent before provider launch",
    "  --grant-skill NAME              Grant an installed Arroba skill to the native agent before provider launch",
    "  creates a new Arroba agent in the selected session and launches native `codex --remote` for it.",
  ].join("\n") + "\n")
}

async function launchManagedNativeProviderRun(options: {
  client: LocalIpcClient
  sessionId: string
  agentId: string
  model: string
  effort: string
}): Promise<RuntimeProviderRun> {
  return requestNativeProviderRunLaunch(options.client, {
    sessionId: options.sessionId,
    provider: "codex",
    model: options.model,
    effort: options.effort,
    agentId: options.agentId,
  })
}

async function runCodexTui(options: {
  proxyUrl: string
  model: string
  workingDirectory: string
  providerSessionId?: string | null
  initialPrompt?: string | undefined
}): Promise<void> {
  const executable = process.env.ARROBA_CODEX_BIN?.trim() || "codex"
  const baseArgs = [
    "--remote",
    options.proxyUrl,
    "--no-alt-screen",
    "--config",
    "check_for_update_on_startup=false",
    "-C",
    options.workingDirectory,
    "-m",
    options.model,
  ]
  const args = options.providerSessionId
    ? ["resume", ...baseArgs, options.providerSessionId]
    : baseArgs
  if (options.initialPrompt) args.push(options.initialPrompt)
  await new Promise<void>((resolve, reject) => {
    const child = spawn(executable, args, {
      stdio: "inherit",
      cwd: options.workingDirectory,
      env: process.env,
    })
    child.once("error", reject)
    child.once("exit", (code, signal) => {
      if (code === 0) {
        resolve()
        return
      }
      reject(new Error(`codex exited with ${signal ?? code}`))
    })
  })
}

function formatError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function debugNativeCodex(label: string, payload: unknown) {
  if (!process.env.ARROBA_CODEX_NATIVE_DEBUG) return
  const line = `[arroba codex native-tui] ${label}: ${JSON.stringify(payload)}\n`
  const debugFile = process.env.ARROBA_CODEX_NATIVE_DEBUG_FILE
  if (debugFile) {
    appendFileSync(debugFile, line)
    return
  }
  process.stderr.write(line)
}
