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

export function applyProviderPreferenceDefaults(options: CliOptions, preferences: ArrobaPreferences): CliOptions {
  const configuredProviderPreferences = preferences.providers?.[options.provider ?? "opencode"]
  if (options.model === "default") {
    options.model = configuredProviderPreferences?.model ?? options.model
  }
  if (!options.effort.trim()) {
    options.effort = configuredProviderPreferences?.effort ?? options.effort
  }
  return options
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
  process.stdout.write([
    "usage: arroba-cli [--detached] [--kernel-url URL] [--socket PATH] [--automation-socket PATH] [--terminal-pairing-link LINK] [--relay-url URL --relay-token TOKEN (--target-daemon-id ID|--target-daemon-alias NAME)] [--session REF] [--create-session] [--alias NAME] [--delete-session REF] [--client-id ID] [--provider NAME] [--model MODEL] [--account-profile PROFILE] [--effort LEVEL] [--workspace PATH] [--worktree PATH]",
    "       arroba-cli logs [--follow] [--process-kind KIND] [--component NAME] [--session ID] [--provider-run ID] [--client-id ID] [--level LEVEL] [--limit N]",
    "",
    "commands:",
    "  /stop                 request cancellation of the active provider turn",
    "  /exit                 exit the CLI",
    "  /waiting              go to the waiting room",
    "  /provider <name>      select the provider backend",
    "  /provider status [n]  show auth status for the current or named provider",
    "  /provider login [n]   start provider-native login for the current or named provider",
    "  /provider logout [n]  clear the current or named provider login",
    "  /provider reauth [n]  log out then start a fresh provider login",
    "  /model <id>           select the active model",
    "  /variant <name>       select the model variant",
    "  /workspace [path]     show or set the next-session workspace path",
    "  /workspace link ...   manage workspace links for the attached session",
    "  /workspace sync ...   manage workspace live sync status, mode, links, conflicts, and ignore rules",
    "  /worktree [path]      show or set the next-session worktree path",
    "  /worktree create <branch> [directory] [--from <ref>] create a named git worktree",
    "  /worktree name [a]    set or clear the current worktree display name",
    "  /view <mode>          set multi-agent response layout to split|individual",
    "  /session new [d]      create and attach to a new session, optionally in directory d",
    "  /session create [d]   alias for /session new",
    "  /session <a>          alias the current session",
    "  /session attach <r>   attach to a session by id or alias",
    "  /session delete [r]   delete the current or referenced session",
    "  /agent spawn [a] [m] [--dir d] [--worktree d --branch b] [--machine r|--slice s] spawn a local, remote, or slice agent",
    "  /agent delete [r]     delete the focused or referenced agent",
    "  /agent destroy [r]    alias for /agent delete",
    "  /agent focus <id>     focus a specific agent",
    "  /agent list           list all agents in the session",
    "  /agent cycle          cycle to the next agent (or use Tab)",
    "  /machine list         list approved, pending, and offline remote machines",
    "  /machine kernels <m>  list live kernels for a remote machine",
    "  /machine approve <m>  approve a pending remote machine for spawning",
    "  /machine forget <m>   forget a registered remote machine",
    "  /machine rename <m> <alias> rename and approve a remote machine",
    "  /slice list           list slices owned by this kernel",
    "  /slice create <n>     create a slice inventory entry",
    "  /slice status [s]     show a slice, defaulting to focused agent slice",
    "  /slice screen [s]     open or print the slice screen URL",
    "  /config show          show the Arroba user config",
    "  /config keys          list settable config keys",
    "  /config schema        show config key metadata",
    "  /config set <p> <v>   update the Arroba user config",
    "  /config workspace-live-sync required|unrestricted set global workspace live sync policy",
    "  /cloud                open Arroba Cloud terminal",
    "  /cloud link           link this machine to Arroba Cloud",
    "  /cloud status         show Cloud and relay status",
    "  /cloud invite create [n] [--level private|transparent|full] create a cloud-backed collaboration invite",
    "  /cloud invite accept <url> accept a collaboration invite",
    "  /cloud members        list cloud members for the session",
    "  /cloud collaborators  list recent cloud collaborators",
    "  /relay status         show relay configuration and connection status",
    "  /relay invite create [n] [--level private|transparent|full] create a same-relay collaboration invite",
    "  /relay invite accept <token> accept a same-relay collaboration invite",
    "  /relay members        list same-relay members for the session",
    "  /relay cloud client <daemon> issue a relay client command",
    "  /collab invite create [n] [--level private|transparent|full] create a collaboration invite using the configured backend",
    "  /collab invite accept <token-or-url> accept a collaboration invite",
    "  /collab members       list collaborators for the current session",
    "  /opencode <cmd>       forward an OpenCode-native command to the focused OpenCode agent",
    "  /codex <cmd>          forward a Codex-native command to the focused Codex agent",
    "  /workflow             open the workflow outline",
    "  /workflow list        list workflows in the workspace",
    "  /workflow show [r]    show selected workflow or workflow by id/alias",
    "  /workflow new [a]     create a new workflow with an optional alias",
    "  /workflow run [w] <e> [p] invoke a workflow endpoint with an optional prompt",
    "  /workflow runs [w]    list workflow runs for the session or one workflow",
    "  /workflow cancel <r>  cancel a workflow run",
    "  /workflow resume <r>  resume a stopped workflow run",
    "  /workflow terminal [w] show the workflow terminal in the I/O panel",
    "  /workflow watchdog ... manage scheduled endpoint triggers",
    "  /workflow <id> <a>    assign an alias to an existing workflow",
    "  /workflow <w> <f> <t> shorthand for /workflow edge add using node ids or agent refs",
    "  /workflow node ...    add/remove workflow nodes; /workflow add node all adds missing agents",
    "  /workflow edge ...    add/remove workflow edges; workflow id may be omitted",
    "  /workflow endpoint ... manage workflow endpoints; workflow id may be omitted",
    "  Tab                   keyboard shortcut to cycle focus",
    "  Ctrl+Tab              switch between the agent screens and workflow outline",
    "",
  ].join("\n"))
}
