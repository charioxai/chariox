import { appendFileSync } from "node:fs"
import { mkdir, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../ipc.js"
import { promptAttachmentTransferIsForced } from "../prompt-attachment-transfer.js"
import { grantNativeCapabilities } from "./capability-grants.js"
import {
  startClaudeBridge,
} from "./claude-bridge.js"
import { claudeHookSettings, writeClaudeHookHandler } from "./claude-hook-handler.js"
import {
  type ClaudePromptOriginState,
  startClaudePermissionBridge,
} from "./claude-permission-bridge.js"
import {
  launchClaudeNativeRun,
  launchClaudeRemoteRenderedRun,
  waitForClaudeProviderRunReady,
  waitForClaudeRemoteRenderedRunExit,
} from "./claude-provider-run.js"
import {
  type ClaudeTuiController,
  startClaudeAttachedPty,
  startClaudeScreen,
} from "./claude-tui-launcher.js"
import {
  createClaudeRemoteRenderedReadiness,
  forwardClaudeRemoteRenderedStdin,
  installClaudeRemoteRenderedResizeForwarder,
  startClaudeRemoteRenderedPumpLoop,
  submitClaudeRemoteRenderedInitialPrompt,
  writeClaudeRemoteRenderedTerminalRecords,
} from "./claude-remote-rendered-io.js"
import { startNativeKernelPumpLoop } from "./native-kernel-pump.js"
import {
  formatNativeTuiRuntimeBanner,
} from "./runtime-banner.js"
import { loadNativeTuiSliceInventory } from "./slice-inventory.js"
import {
  defaultKernelEndpoint,
  inferWorkspaceTargetsFromLaunchDirectory,
  parseKernelPort,
  parseNativeMode as parseMode,
  parseNativePermissions as parsePermissions,
} from "./launch-environment.js"
import {
  attachNativeSession,
  createNativeSession,
  prepareCreatedNativeAgent,
  resolveNativeSession,
  spawnNativeAgent,
} from "./session-control.js"

type NativeClaudeOptions = {
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
  detachedScreen: boolean
  remoteRendered: boolean
  grantMcps: string[]
  grantSkills: string[]
}

export async function runClaudeNativeTui(args: string[]): Promise<void> {
  const options = parseNativeClaudeArgs(args)
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
  if (options.remoteRendered) {
    await runClaudeRemoteRendered(options, client, workspace, worktree)
    return
  }

  const tempRoot = path.join(os.tmpdir(), `arroba-claude-native-${process.pid}-${Date.now()}`)
  const eventsFile = path.join(tempRoot, "events.jsonl")
  const contextFile = path.join(tempRoot, "hidden-context.txt")
  const contextResponseDir = path.join(tempRoot, "hook-context-responses")
  const attachmentContextDir = path.join(tempRoot, "attachments")
  const settingsPath = path.join(tempRoot, "settings.json")
  const hookHandlerPath = path.join(tempRoot, "hook-handler.mjs")
  const screenName = `arroba-claude-${process.pid}-${Date.now()}`
  const screenLogDir = path.join(tempRoot, "screen")
  let bridge: { stop: () => void } | null = null
  let permissionBridge: { url: string; stop: () => Promise<void> } | null = null
  let pump: { stop: () => void } | null = null
  let tui: ClaudeTuiController | null = null
  const promptOrigin: ClaudePromptOriginState = { current: null }

  try {
    await mkdir(screenLogDir, { recursive: true })
    await mkdir(contextResponseDir, { recursive: true })
    await writeFile(contextFile, "", "utf8")
    await writeClaudeHookHandler(hookHandlerPath)
    await writeFile(settingsPath, JSON.stringify(claudeHookSettings(hookHandlerPath), null, 2), "utf8")

    const created = options.sessionRef
      ? null
      : await createNativeSession(client, workspace, worktree, options.alias, {
        provider: "claude",
        model: options.model,
        effort: options.effort,
        execution_mode: options.mode,
        permission_level: options.permissions,
      })
    const session = created?.session ?? await resolveNativeSession(client, options.sessionRef!, workspace)
    const attachment = await attachNativeSession(client, session.id, options.clientId)
    const agent = created?.agent
      ? await prepareCreatedNativeAgent(client, session.id, created.agent, options.agentAlias, options.machineRef)
      : await spawnNativeAgent(client, session.id, "claude", options.agentAlias, options.model, worktree, options.effort, options.mode, options.permissions, options.machineRef)
    await grantNativeCapabilities(client, workspace, agent.id, options.grantMcps, options.grantSkills)
    const launched = await launchClaudeNativeRun(client, session.id, agent.id, options.model, options.effort)
    const run = launched.session_id === session.id
      ? await waitForClaudeProviderRunReady(client, launched.id)
      : launched
    permissionBridge = await startClaudePermissionBridge({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      promptOrigin,
    })

    const sliceInventory = agent.remote_execution
      ? await loadNativeTuiSliceInventory(client)
      : { slices: [], error: null }
    process.stderr.write(formatNativeTuiRuntimeBanner({
      surface: "claude native-tui",
      session,
      agent,
      worktree,
      slices: sliceInventory.slices,
      sliceLookupError: options.sliceRef ? sliceInventory.error : null,
      run,
      grantedMcps: options.grantMcps,
      grantedSkills: options.grantSkills,
      providerLines: [
        `  tui:            ${options.detachedScreen ? `screen:${screenName}` : "attached-pty"}`,
        ...(options.detachedScreen ? [`  screen:         ${screenName}`] : []),
      ],
      promptPolicy: "Claude Code TUI is native; Arroba observes hooks and injects queued prompts through the PTY",
    }))

    const launchOptions: Parameters<typeof startClaudeScreen>[2] = {
      worktree,
      settingsPath,
      model: options.model,
      effort: options.effort,
      permissions: options.permissions,
      env: {
        ...process.env,
        ARROBA_CLAUDE_NATIVE_EVENTS: eventsFile,
        ARROBA_CLAUDE_NATIVE_CONTEXT: contextFile,
        ARROBA_CLAUDE_NATIVE_CONTEXT_RESPONSES: contextResponseDir,
        ARROBA_CLAUDE_NATIVE_HOOK_BRIDGE_URL: permissionBridge.url,
      },
    }
    tui = options.detachedScreen
      ? await startClaudeScreen(screenName, screenLogDir, launchOptions)
      : await startClaudeAttachedPty(launchOptions)
    bridge = startClaudeBridge({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      providerRunId: run.id,
      eventsFile,
      contextFile,
      attachmentContextDir,
      hookContextResponseDir: contextResponseDir,
      workspace,
      worktree,
      inlineLocalAttachments: Boolean(options.relayUrl) || promptAttachmentTransferIsForced(),
      promptOrigin,
      submitPrompt: tui.submitPrompt,
      debug: debugNativeClaude,
    })
    pump = startNativeKernelPumpLoop(client, session.id, attachment.id)
    if (options.initialPrompt) {
      await sleep(1_000)
      await tui.submitPrompt(options.initialPrompt)
    }

    await tui.waitForExit()
  } finally {
    bridge?.stop()
    pump?.stop()
    await tui?.stop()
    await permissionBridge?.stop().catch(() => {})
    await client.close()
    if (!options.detachedScreen) {
      await rm(tempRoot, { recursive: true, force: true }).catch(() => {})
    }
  }
}

function parseNativeClaudeArgs(args: string[]): NativeClaudeOptions {
  const options: NativeClaudeOptions = {
    clientId: `arroba-claude-native-${process.pid}`,
    model: "claude-sonnet-4-6",
    effort: "low",
    mode: "build",
    permissions: "required",
    detachedScreen: false,
    remoteRendered: false,
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
      case "--mode":
        options.mode = parseMode(next())
        break
      case "--permissions":
        options.permissions = parsePermissions(next())
        break
      case "--initial-prompt":
        options.initialPrompt = next()
        break
      case "--detached-screen":
        options.detachedScreen = true
        break
      case "--remote-rendered":
        options.remoteRendered = true
        break
      case "--grant-mcp":
        options.grantMcps.push(next())
        break
      case "--grant-skill":
        options.grantSkills.push(next())
        break
      case "--help":
      case "-h":
        printNativeClaudeUsage()
        process.exit(0)
      default:
        if (arg.startsWith("-")) throw new Error(`unknown claude argument ${arg}`)
        positional.push(arg)
    }
  }
  if (positional.length > 1) throw new Error(`unexpected claude arguments: ${positional.slice(1).join(" ")}`)
  if (options.machineRef && options.sliceRef) {
    throw new Error("--machine and --slice cannot be used together")
  }
  if (options.machineRef && !options.remoteRendered) {
    throw new Error("--machine requires --remote-rendered so Claude Code runs in the worker kernel PTY")
  }
  if (options.sliceRef && !options.remoteRendered) {
    throw new Error("--slice requires --remote-rendered so Claude Code runs in the slice worker kernel PTY")
  }
  if (positional[0] !== undefined) options.sessionRef = positional[0]
  return options
}

function printNativeClaudeUsage() {
  process.stdout.write([
    "Usage: arroba claude [session_id] [options]",
    "",
    "Launch Claude Code's native TUI as an Arroba-managed native provider agent.",
    "",
    "Options:",
    "  --kernel-port, --port <port>     Kernel websocket port (default 43119)",
    "  --kernel-url <url>               Kernel websocket URL",
    "  --workspace <path>               Workspace root",
    "  --worktree <path>                Worktree root",
    "  --machine, --kernel-ref <ref>    Run the Arroba agent/provider on a remote worker kernel",
    "  --slice <ref>                    Run the Arroba agent/provider on a home-managed slice worker",
    "  --alias <name>                   Alias for a newly-created session",
    "  --agent-alias <name>             Alias for the Claude native agent",
    "  --model <model>                  Claude model argument (default claude-sonnet-4-6)",
    "  --effort <effort>                Claude effort argument (default low)",
    "  --mode <build|plan>              Arroba agent mode (default build)",
    "  --permissions <required|yolo>    Claude permission mode mapping (default required)",
    "  --remote-rendered                Run Claude Code in the target kernel PTY and render it here",
    "  --grant-mcp <name>               Grant an installed Arroba MCP to the native agent before provider launch",
    "  --grant-skill <name>             Grant an installed Arroba skill to the native agent before provider launch",
    "",
  ].join("\n"))
}

async function runClaudeRemoteRendered(
  options: NativeClaudeOptions,
  client: LocalIpcClient,
  workspace: string,
  worktree: string,
): Promise<void> {
  let pump: { stop: () => void } | null = null
  let disposeEvents: (() => void) | null = null
  let restoreStdin: (() => void) | null = null
  let restoreResize: (() => void) | null = null
  try {
    const created = options.sessionRef
      ? null
      : await createNativeSession(client, workspace, worktree, options.alias, {
        provider: "claude",
        model: options.model,
        effort: options.effort,
        execution_mode: options.mode,
        permission_level: options.permissions,
      }, options.sliceRef)
    const session = created?.session ?? await resolveNativeSession(client, options.sessionRef!, workspace)
    const attachment = await attachNativeSession(client, session.id, options.clientId)
    const agent = created?.agent
      ? await prepareCreatedNativeAgent(client, session.id, created.agent, options.agentAlias, options.machineRef)
      : await spawnNativeAgent(client, session.id, "claude", options.agentAlias, options.model, worktree, options.effort, options.mode, options.permissions, options.machineRef, options.sliceRef)
    await grantNativeCapabilities(client, workspace, agent.id, options.grantMcps, options.grantSkills)
    const launched = await launchClaudeRemoteRenderedRun(client, session.id, agent.id, options.model, options.effort)
    const run = await waitForClaudeProviderRunReady(client, launched.id)
    const readiness = createClaudeRemoteRenderedReadiness()

    const sliceInventory = agent.remote_execution
      ? await loadNativeTuiSliceInventory(client)
      : { slices: [], error: null }
    process.stderr.write(formatNativeTuiRuntimeBanner({
      surface: "claude remote-native-tui",
      session,
      agent,
      worktree,
      slices: sliceInventory.slices,
      sliceLookupError: options.sliceRef ? sliceInventory.error : null,
      run,
      grantedMcps: options.grantMcps,
      grantedSkills: options.grantSkills,
      providerLines: ["  tui:            target-kernel-pty"],
    }))

    disposeEvents = client.onKernelEvent((event) => {
      if (event.event !== "terminal_output") return
      readiness.observe(event.records, run.id, agent.id)
      writeClaudeRemoteRenderedTerminalRecords(event.records, run.id, agent.id)
    })
    await client.subscribeToKernelEvents(session.id, attachment.id)
    pump = startClaudeRemoteRenderedPumpLoop(
      client,
      session.id,
      attachment.id,
      run.id,
      agent.id,
      (records) => readiness.observe(records, run.id, agent.id),
    )
    restoreStdin = forwardClaudeRemoteRenderedStdin({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      providerRunId: run.id,
      worktree,
      inlineLocalAttachments: Boolean(options.relayUrl) || Boolean(options.machineRef) || Boolean(options.sliceRef) || promptAttachmentTransferIsForced(),
      debug: debugNativeClaude,
    })
    restoreResize = installClaudeRemoteRenderedResizeForwarder(client, session.id, run.id)
    if (options.initialPrompt) {
      await submitClaudeRemoteRenderedInitialPrompt({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        providerRunId: run.id,
        prompt: options.initialPrompt,
        readiness,
      })
    }
    await waitForClaudeRemoteRenderedRunExit(client, run.id)
  } finally {
    restoreResize?.()
    restoreStdin?.()
    disposeEvents?.()
    pump?.stop()
    await client.unsubscribeFromKernelEvents().catch(() => {})
    await client.close()
  }
}

function debugNativeClaude(label: string, payload: unknown) {
  if (!process.env.ARROBA_CLAUDE_NATIVE_DEBUG) return
  const line = `[arroba claude native-tui] ${label}: ${JSON.stringify(payload)}\n`
  const debugFile = process.env.ARROBA_CLAUDE_NATIVE_DEBUG_FILE
  if (debugFile) {
    appendFileSync(debugFile, line)
    return
  }
  process.stderr.write(line)
}
