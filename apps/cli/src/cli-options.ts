import path from "node:path"
import process from "node:process"

import type {
  ArrobaPreferences,
} from "./preferences.js"
import type {
  CliOptions,
} from "./cli-types.js"
import { describeCliError } from "./runtime.js"

export function parseArgs(args: string[]): CliOptions {
  const options: CliOptions = {
    clientId: `arroba-cli-${process.pid}`,
    provider: "opencode",
    model: "default",
    accountProfile: "default",
    effort: "",
  }

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
      case "--detached":
        options.detached = true
        break
      case "--socket":
        options.socketPath = next()
        break
      case "--automation-socket":
        options.automationSocket = path.resolve(next())
        break
      case "--kernel-url":
        options.kernelUrl = next()
        break
      case "--session":
        options.sessionId = next()
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
      case "--terminal-pairing-link":
      case "--pairing-link":
        applyTerminalPairingLinkOptions(options, next())
        break
      case "--create-session":
        options.createSession = true
        break
      case "--delete-session":
        options.deleteSessionRef = next()
        break
      case "--alias":
        options.alias = next()
        break
      case "--client-id":
        options.clientId = next()
        break
      case "--model":
        options.model = next()
        break
      case "--provider":
        options.provider = next()
        break
      case "--account-profile":
        options.accountProfile = next()
        break
      case "--effort":
        options.effort = next()
        break
      case "--workspace":
        options.workspace = path.resolve(next())
        break
      case "--worktree":
        options.worktree = path.resolve(next())
        break
      case "--help":
      case "-h":
        printUsage()
        process.exit(0)
      default:
        if (isTerminalPairingLink(arg)) {
          applyTerminalPairingLinkOptions(options, arg)
          break
        }
        throw new Error(`unknown argument ${arg}`)
    }
  }

  validateOptions(options)
  return options
}

export function defaultKernelEndpoint(): string {
  if (process.env.ARROBA_KERNEL_URL) {
    return process.env.ARROBA_KERNEL_URL
  }
  const host = process.env.ARROBA_KERNEL_HOST ?? "127.0.0.1"
  const port = process.env.ARROBA_KERNEL_PORT ?? "43118"
  return `ws://${host}:${port}/kernel`
}

export function resolveConfiguredCloudRelayApiUrl(preferences: ArrobaPreferences): string | undefined {
  const configured = process.env.ARROBA_CLOUD_API_URL
    ?? process.env.ARROBA_CLOUD_HOSTED_API_URL
    ?? preferences.relay?.cloud?.apiUrl
  return configured?.trim().replace(/\/+$/, "") || undefined
}

function validateOptions(options: CliOptions): void {
  if (options.createSession && options.sessionId) {
    throw new Error("--create-session cannot be used together with --session")
  }
  if (options.detached && (options.createSession || options.sessionId || options.deleteSessionRef)) {
    throw new Error("--detached cannot be used together with --create-session, --session, or --delete-session")
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
  if (options.createSession && options.deleteSessionRef) {
    throw new Error("--create-session cannot be used together with --delete-session")
  }
  if (options.alias && !options.createSession) {
    throw new Error("--alias requires --create-session")
  }
}

function isTerminalPairingLink(value: string) {
  return value.trim().startsWith("arroba-terminal-pair-v1.")
}

function applyTerminalPairingLinkOptions(options: CliOptions, pairingLink: string) {
  const parsed = parseTerminalPairingLink(pairingLink)
  options.relayUrl = parsed.relayUrl
  options.relayToken = parsed.relayToken
  options.targetDaemonId = parsed.targetDaemonId
  if (parsed.targetDaemonAlias) {
    options.targetDaemonAlias = parsed.targetDaemonAlias
  }
  options.clientId = parsed.terminalId ?? options.clientId
}

function parseTerminalPairingLink(pairingLink: string) {
  const payload = pairingLink.trim().replace(/^arroba-terminal-pair-v1[.]/, "")
  let decoded: Record<string, unknown>
  try {
    decoded = JSON.parse(Buffer.from(payload, "base64url").toString("utf8")) as Record<string, unknown>
  } catch (error) {
    throw new Error(`invalid terminal pairing link: ${describeCliError(error)}`)
  }
  const relayUrl = typeof decoded.relay_url === "string" ? decoded.relay_url : ""
  const relayToken = typeof decoded.relay_token === "string" ? decoded.relay_token : ""
  const targetDaemonId = typeof decoded.target_daemon_id === "string" ? decoded.target_daemon_id : ""
  if (!relayUrl || !relayToken || !targetDaemonId) {
    throw new Error("invalid terminal pairing link: missing relay target details")
  }
  return {
    relayUrl,
    relayToken,
    targetDaemonId,
    targetDaemonAlias: typeof decoded.target_daemon_alias === "string" ? decoded.target_daemon_alias : undefined,
    terminalId: typeof decoded.terminal_id === "string" ? decoded.terminal_id : undefined,
  }
}

function printUsage() {
  process.stdout.write(
    "usage: arroba-cli [--detached] [--kernel-url URL] [--socket PATH] [--automation-socket PATH] [--terminal-pairing-link LINK] [--relay-url URL --relay-token TOKEN (--target-daemon-id ID|--target-daemon-alias NAME)] [--session REF] [--create-session] [--alias NAME] [--delete-session REF] [--client-id ID] [--provider NAME] [--model MODEL] [--account-profile PROFILE] [--effort LEVEL] [--workspace PATH] [--worktree PATH]\n       arroba-cli logs [--follow] [--process-kind KIND] [--component NAME] [--session ID] [--provider-run ID] [--client-id ID] [--level LEVEL] [--limit N]\n\ncommands:\n  /stop                 request cancellation of the active provider turn\n  /exit                 exit the CLI\n  /waiting              go to the waiting room\n  /provider <name>      select the provider backend\n  /provider status [n]  show auth status for the current or named provider\n  /provider login [n]   start provider-native login for the current or named provider\n  /provider logout [n]  clear the current or named provider login\n  /provider reauth [n]  log out then start a fresh provider login\n  /model <id>           select the active model\n  /variant <name>       select the model variant\n  /workspace [path]     show or set the next-session workspace path\n  /workspace link ...   manage workspace links for the attached session\n  /worktree [path]      show or set the next-session worktree path\n  /worktree create <branch> [directory] [--from <ref>] create a named git worktree\n  /worktree name [a]    set or clear the current worktree display name\n  /view <mode>          set multi-agent response layout to split|individual\n  /session new [d]      create and attach to a new session, optionally in directory d\n  /session create [d]   alias for /session new\n  /session <a>          alias the current session\n  /session attach <r>   attach to a session by id or alias\n  /session delete [r]   delete the current or referenced session\n  /agent spawn [a] [m] [--dir d] [--worktree d --branch b] [--machine r|--slice s] spawn a local, remote, or slice agent\n  /agent delete [r]     delete the focused or referenced agent\n  /agent destroy [r]    alias for /agent delete\n  /agent focus <id>     focus a specific agent\n  /agent list           list all agents in the session\n  /agent cycle          cycle to the next agent (or use Tab)\n  /machine list         list approved, pending, and offline remote machines\n  /machine kernels <m>  list live kernels for a remote machine\n  /machine approve <m>  approve a pending remote machine for spawning\n  /machine forget <m>   forget a registered remote machine\n  /machine rename <m> <alias> rename and approve a remote machine\n  /slice list           list slices owned by this kernel\n  /slice create <n>     create a slice inventory entry\n  /slice status [s]     show a slice, defaulting to focused agent slice\n  /slice screen [s]     open or print the slice screen URL\n  /config show          show the Arroba user config\n  /config keys          list settable config keys\n  /config schema        show config key metadata\n  /config set <p> <v>   update the Arroba user config\n  /config managed-io required|unrestricted set global managed I/O\n  /opencode <cmd>       forward an OpenCode-native command to the focused OpenCode agent\n  /codex <cmd>          forward a Codex-native command to the focused Codex agent\n  /workflow             open the workflow outline\n  /workflow list        list workflows in the workspace\n  /workflow show [r]    show selected workflow or workflow by id/alias\n  /workflow new [a]     create a new workflow with an optional alias\n  /workflow run [w] <e> [p] invoke a workflow endpoint with an optional prompt\n  /workflow runs [w]    list workflow runs for the session or one workflow\n  /workflow cancel <r>  cancel a workflow run\n  /workflow resume <r>  resume a stopped workflow run\n  /workflow terminal [w] show the workflow terminal in the I/O panel\n  /workflow watchdog ... manage scheduled endpoint triggers\n  /workflow <id> <a>    assign an alias to an existing workflow\n  /workflow <w> <f> <t> shorthand for /workflow edge add using node ids or agent refs\n  /workflow node ...    add/remove workflow nodes; /workflow add node all adds missing agents\n  /workflow edge ...    add/remove workflow edges; workflow id may be omitted\n  /workflow endpoint ... manage workflow endpoints; workflow id may be omitted\n  Tab                   keyboard shortcut to cycle focus\n  Ctrl+Tab              switch between the agent screens and workflow outline\n",
  )
}
