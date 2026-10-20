import { localKernelConnectionLabel } from "../local-connection-indicator.js"
import { appendFileSync, renameSync, writeFileSync } from "node:fs"
import path from "node:path"
import process from "node:process"

import { LocalIpcClient } from "../ipc.js"
import { createCliRelayIdentityStore } from "../cli-relay-identity-store.js"
import { promptAttachmentTransferIsForced } from "../prompt-attachment-transfer.js"
import { runClaudeKernelScreen } from "./claude-kernel-screen.js"
import { grantNativeCapabilities } from "./capability-grants.js"
import {
  launchClaudeRemoteRenderedRun,
  waitForClaudeProviderRunReady,
  waitForClaudeRemoteRenderedRunExit,
} from "./claude-provider-run.js"
import {
  createClaudeRemoteRenderedReadiness,
  forwardClaudeRemoteRenderedStdin,
  installClaudeRemoteRenderedResizeForwarder,
  startClaudeRemoteRenderedPumpLoop,
  submitClaudeRemoteRenderedInitialPrompt,
  writeClaudeRemoteRenderedTerminalRecords,
} from "./claude-remote-rendered-io.js"
import {
  formatNativeTuiRuntimeBanner,
} from "./runtime-banner.js"
import { loadNativeTuiSliceInventory } from "./slice-inventory.js"
import {
  defaultKernelEndpoint,
  inferWorkspaceTargetsFromLaunchDirectory,
  parseKernelPort,
  relayTokenFromEnvironment,
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
  relayTokenIssuer?: { endpoint: string; daemonId: string }
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
  detachedScreenIndices: number[]
  grantMcps: string[]
  grantSkills: string[]
}

export async function runClaudeNativeTui(args: string[]): Promise<void> {
  const options = parseNativeClaudeArgs(args)
  const inferredTargets = await inferWorkspaceTargetsFromLaunchDirectory(process.cwd())
  const workspace = options.workspace ?? inferredTargets.workspace
  const worktree = options.worktree ?? inferredTargets.worktree
  if (options.detachedScreen) {
    await runClaudeKernelScreen(args.filter((_, index) => !options.detachedScreenIndices.includes(index)), worktree)
    return
  }
  const kernelEndpoint = options.relayUrl ?? options.kernelUrl ?? options.socketPath ?? defaultKernelEndpoint(options.kernelPort)
  const client = new LocalIpcClient(kernelEndpoint, options.relayUrl
    ? {
      relayAuthToken: options.relayToken,
      relayIdentity: createCliRelayIdentityStore().load() ?? undefined,
      relayAuthorizationIssuer: options.relayTokenIssuer,
      targetDaemonId: options.targetDaemonId,
      targetDaemonAlias: options.targetDaemonAlias,
    }
    : undefined)
  // Local and remote native input share the home kernel's provider run,
  // prompt admission, permission interactions and hidden-context hook.
  await runClaudeRemoteRendered(options, client, workspace, worktree)
}

function parseNativeClaudeArgs(args: string[]): NativeClaudeOptions {
  const options: NativeClaudeOptions = {
    clientId: `chariox-claude-native-${process.pid}`,
    model: "claude-sonnet-4-6",
    effort: "low",
    mode: "build",
    permissions: "required",
    detachedScreen: false,
    detachedScreenIndices: [],
    grantMcps: [],
    grantSkills: [],
  }
  let relayTokenSource = false
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
      case "--relay-token-issuer":
        options.relayTokenIssuer = { endpoint: next(), daemonId: next() }
        break
      case "--relay-token":
        if (relayTokenSource) throw new Error("--relay-token and --relay-token-env cannot be combined")
        relayTokenSource = true
        options.relayToken = next()
        break
      case "--relay-token-env":
        if (relayTokenSource) throw new Error("--relay-token and --relay-token-env cannot be combined")
        relayTokenSource = true
        options.relayToken = relayTokenFromEnvironment(next())
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
        options.detachedScreenIndices.push(index)
        break
      case "--remote-rendered":
        // Retained compatibility flag; the kernel-rendered path is the default.
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
  if (positional[0] !== undefined) options.sessionRef = positional[0]
  return options
}

function printNativeClaudeUsage() {
  process.stdout.write([
    "Usage: chariox claude [session_id] [options]",
    "",
    "Launch Claude Code's native TUI as a Chariox-managed native provider agent.",
    "",
    "Options:",
    "  --kernel-port, --port <port>     Kernel websocket port (default 43119)",
    "  --kernel-url <url>               Kernel websocket URL",
    "  --relay-url <url>                Hosted relay websocket URL",
    "  --relay-token-env <name>         Read relay authorization from the named environment variable",
    "  --workspace <path>               Workspace root",
    "  --worktree <path>                Worktree root",
    "  --machine, --kernel-ref <ref>    Run the Chariox agent/provider on a remote worker kernel",
    "  --slice <ref>                    Run the Chariox agent/provider on a home-managed slice worker",
    "  --alias <name>                   Alias for a newly-created session",
    "  --agent-alias <name>             Alias for the Claude native agent",
    "  --model <model>                  Claude model argument (default claude-sonnet-4-6)",
    "  --effort <effort>                Claude effort argument (default low)",
    "  --mode <build|plan>              Chariox agent mode (default build)",
    "  --permissions <required|yolo>    Claude permission mode mapping (default required)",
    "  --remote-rendered                Run Claude Code in the target kernel PTY and render it here",
    "  --grant-mcp <name>               Grant an installed Chariox MCP to the native agent before provider launch",
    "  --grant-skill <name>             Grant an installed Chariox skill to the native agent before provider launch",
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
  const exitController = new AbortController()
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
    // MP-08/MP-11: trust/onboarding can end the official provider. Release the
    // queued initial prompt wait and its timer instead of waiting sixty seconds.
    const exited = waitForClaudeRemoteRenderedRunExit(client, run.id, exitController.signal)
    void exited.then(() => readiness.fail(new Error(
      "Claude native TUI ended before its input composer was ready. Complete onboarding and trust the workspace, then retry.",
    )), (error: unknown) => readiness.fail(error instanceof Error ? error : new Error(String(error))))

    const sliceInventory = agent.remote_execution
      ? await loadNativeTuiSliceInventory(client)
      : { slices: [], error: null }
    const banner = formatNativeTuiRuntimeBanner({
      connectionLabel: localKernelConnectionLabel(client, session.host_daemon_id),
      surface: "claude native-tui",
      session,
      agent,
      worktree,
      slices: sliceInventory.slices,
      sliceLookupError: options.sliceRef ? sliceInventory.error : null,
      run,
      grantedMcps: options.grantMcps,
      grantedSkills: options.grantSkills,
      providerLines: ["  tui:            target-kernel-pty"],
    })
    process.stderr.write(banner)
    if (process.env.CHARIOX_CLAUDE_NATIVE_BANNER_FILE) {
      const destination = process.env.CHARIOX_CLAUDE_NATIVE_BANNER_FILE
      writeFileSync(`${destination}.tmp`, banner, { mode: 0o600 })
      renameSync(`${destination}.tmp`, destination)
    }

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
    await exited
  } finally {
    exitController.abort()
    restoreResize?.()
    restoreStdin?.()
    disposeEvents?.()
    pump?.stop()
    await client.unsubscribeFromKernelEvents().catch(() => {})
    await client.close()
  }
}

function debugNativeClaude(label: string, payload: unknown) {
  if (!process.env.CHARIOX_CLAUDE_NATIVE_DEBUG) return
  const line = `[chariox claude native-tui] ${label}: ${JSON.stringify(payload)}\n`
  const debugFile = process.env.CHARIOX_CLAUDE_NATIVE_DEBUG_FILE
  if (debugFile) {
    appendFileSync(debugFile, line)
    return
  }
  process.stderr.write(line)
}
