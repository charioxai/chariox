import type { ParsedSlashCommand } from "./commands.js"
import type { RelayCloudProfile } from "./preferences.js"
import {
  handleCloudSessionCommand,
  parseCollaborationLevel,
  type CloudSessionCommandHandlerDeps,
} from "./cloud-session-command-handlers.js"
import {
  buildHostedCloudTerminalUrl,
  openHostedCloud,
  startHostedCloudLink,
  type CloudCommandLifecycleDeps,
  type RelayStatus,
} from "./cloud-command-lifecycle.js"
import {
  handleRelayCloudCommand,
  type RelayCloudCommandHandlerDeps,
} from "./relay-cloud-command-handlers.js"
import { handleDeployedWorkflowCloudCommand } from "./deployed-workflow-command.js"

export type CloudCommandHandlerDeps =
  & CloudCommandLifecycleDeps
  & CloudSessionCommandHandlerDeps
  & RelayCloudCommandHandlerDeps

export async function handleRelaySlashCommand(
  deps: CloudCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "relay" }>,
): Promise<void> {
  const [subcommand, ...args] = command.args
  if (subcommand === "cloud") {
    await handleRelayCloudCommand(deps, args)
    return
  }
  if (subcommand === "invite") {
    await handleRelayInviteCommand(deps, actionArg(command.args), restArgs(command.args))
    return
  }
  if ((subcommand === "members" && !args[0]) || (subcommand === "members" && args[0] === "list")) {
    await listRelayMembers(deps)
    return
  }
  if (!subcommand || subcommand === "status") {
    if (!deps.getRelayStatus) {
      deps.flashFooter("relay status is unavailable in this build", "error")
      return
    }
    const status = await deps.getRelayStatus()
    const state = !status.configured ? "not configured" : status.connected ? "connected" : "configured, disconnected"
    deps.appendNotice(
      `relay ${state}\nurl=${status.relay_url ?? "-"}\ntoken_configured=${String(status.relay_token_configured)}\ndaemon=${status.daemon_id}\nmachine=${status.machine_alias ?? status.machine_id}`,
    )
    deps.flashFooter(`relay ${state}`, "info")
    return
  }
  if (subcommand === "use" || subcommand === "configure") {
    if (!deps.configureRelay) {
      deps.flashFooter("relay configuration is unavailable in this build", "error")
      return
    }
    const relayUrl = args[0]
    const relayToken = args[1] ?? process.env.ARROBA_RELAY_TOKEN
    if (!relayUrl) {
      deps.flashFooter("usage: /relay use <ws-url> [token]", "error")
      return
    }
    if (!relayToken) {
      deps.flashFooter("relay token missing; pass it or set ARROBA_RELAY_TOKEN", "error")
      return
    }
    const status = await deps.configureRelay(relayUrl, relayToken)
    await deps.refreshWaitingRoomData?.()
    deps.flashFooter(
      `relay configured: ${status.relay_url ?? relayUrl} (${status.connected ? "connected" : "connecting"})`,
      "info",
    )
    return
  }
  if (subcommand === "disable" || subcommand === "reset") {
    if (!deps.configureRelay) {
      deps.flashFooter("relay configuration is unavailable in this build", "error")
      return
    }
    await deps.configureRelay(null, null)
    await deps.refreshWaitingRoomData?.()
    deps.flashFooter("relay disabled", "info")
    return
  }
  deps.flashFooter("usage: /relay status | /relay use <ws-url> [token] | /relay disable | /relay invite create [max-uses] [--level private|transparent|full] | /relay invite accept <token> | /relay members", "error")
}

export async function handleCloudSlashCommand(
  deps: CloudCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "cloud" }>,
): Promise<void> {
  const [area, action, ...args] = command.args
  if (!area || area === "open") {
    await openHostedCloud(deps)
    return
  }
  if (area === "link" || area === "login") {
    await startHostedCloudLink(deps)
    return
  }
  const profile = deps.getCloudRelayProfile?.() ?? null
  if (area === "status") {
    await showCloudStatus(deps, profile)
    return
  }
  if (!profile) {
    deps.flashFooter("cloud profile missing; run /cloud link first", "error")
    return
  }
  if (await handleDeployedWorkflowCloudCommand(deps, profile, area, action, args)) {
    return
  }
  if (await handleCloudSessionCommand(deps, profile, area, action, args)) {
    return
  }
  deps.flashFooter("usage: /cloud [open|link|status] | /cloud deployments list|show|create|adopt|preflight|release|promote|rollback|start|stop|restart|usage|limits|credentials | /cloud invite create|accept | /cloud members | /cloud collaborators", "error")
}

export async function handleCollabSlashCommand(
  deps: CloudCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "collab" }>,
): Promise<void> {
  const profile = deps.getCloudRelayProfile?.() ?? null
  if (profile) {
    await handleCloudSlashCommand(deps, {
      kind: "cloud",
      raw: command.raw.replace(/^\/collab/, "/cloud"),
      args: command.args,
    })
    return
  }

  const relayStatus = await deps.getRelayStatus?.()
  if (relayStatus?.configured) {
    await handleRelaySlashCommand(deps, {
      kind: "relay",
      raw: command.raw.replace(/^\/collab/, "/relay"),
      args: command.args,
    })
    return
  }

  deps.flashFooter("collaboration invites require Cloud or a relay-connected session", "error")
}

async function showCloudStatus(deps: CloudCommandHandlerDeps, profile: RelayCloudProfile | null): Promise<void> {
  if (!profile) {
    appendCloudNotice(deps, "Cloud is not linked.\nRun /cloud link to connect this machine.")
    deps.flashFooter("cloud not linked", "info")
    return
  }
  const relayStatus = await deps.getRelayStatus?.()
  const relayState = relayStateLabel(relayStatus)
  const lines = [
    "Cloud linked.",
    `account=${profile.accountSlug}`,
    `email=${profile.email}`,
    `url=${buildHostedCloudTerminalUrl(profile.apiUrl)}`,
    `relay=${relayState}`,
  ]
  if (relayStatus) {
    lines.push(`relay_url=${relayStatus.relay_url ?? profile.relayUrl ?? "-"}`)
    lines.push(`machine=${relayStatus.machine_id}`)
  }
  if (relayStatus && !relayStatus.connected) {
    lines.push("kernel=offline in Cloud")
    lines.push("next=keep this CLI running; run /cloud link if the link was revoked or /relay cloud connect after the relay is reachable.")
  }
  appendCloudNotice(deps, lines.join("\n"))
  deps.flashFooter(`cloud linked: ${profile.accountSlug}`, "info")
}

function relayStateLabel(status: RelayStatus | null | undefined): string {
  if (!status) return "unknown"
  if (!status.configured) return "not configured"
  return status.connected ? "connected" : "not connected"
}

function appendCloudNotice(deps: CloudCommandHandlerDeps, message: string): void {
  ;(deps.appendCloudNotice ?? deps.appendNotice)(message)
}

function actionArg(args: readonly string[]): string | undefined {
  return args[1]
}

function restArgs(args: readonly string[]): string[] {
  return [...args.slice(2)]
}

async function handleRelayInviteCommand(
  deps: CloudCommandHandlerDeps,
  action: string | undefined,
  args: string[],
): Promise<void> {
  if (action === "create") {
    await createRelayInvite(deps, args)
    return
  }
  if (action === "accept") {
    await acceptRelayInvite(deps, args)
    return
  }
  deps.flashFooter("usage: /relay invite create [max-uses] [--level private|transparent|full] | /relay invite accept <token>", "error")
}

async function createRelayInvite(deps: CloudCommandHandlerDeps, args: string[]): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("attach to a session before creating a relay invite", "error")
    return
  }
  if (!deps.createSessionInvite) {
    deps.flashFooter("relay invite creation is unavailable in this build", "error")
    return
  }
  const collaborationLevel = parseCollaborationLevel(args)
  if (!collaborationLevel) {
    deps.flashFooter("usage: /relay invite create [max-uses|--max-uses n] [--level private|transparent|full]", "error")
    return
  }
  const maxUsesIndex = args.findIndex((value) => value === "--max-uses")
  const maxUses = parsePositiveInt(maxUsesIndex >= 0 ? args[maxUsesIndex + 1] : args[0]) ?? 1
  if (maxUses <= 0) {
    deps.flashFooter("usage: /relay invite create [max-uses|--max-uses n] [--level private|transparent|full]", "error")
    return
  }
  const local = await deps.createSessionInvite(deps.sessionState().id, null, maxUses, collaborationLevel)
  deps.applySessionState(local.session)
  deps.appendNotice([
    "relay session invite",
    `invite_token=${local.invite.invite_token}`,
    `level=${collaborationLevel}`,
    "share this token with a user already connected to the same relay",
  ].join("\n"))
  deps.flashFooter("relay invite created", "info")
}

async function acceptRelayInvite(deps: CloudCommandHandlerDeps, args: string[]): Promise<void> {
  if (!deps.joinSessionInvite) {
    deps.flashFooter("relay invite acceptance is unavailable in this build", "error")
    return
  }
  const inviteToken = args[0]
  if (!inviteToken) {
    deps.flashFooter("usage: /relay invite accept <token>", "error")
    return
  }
  const relayStatus = await deps.getRelayStatus?.()
  const userId = relayStatus?.daemon_id || "relay-user"
  const joined = await deps.joinSessionInvite(inviteToken, userId)
  deps.applySessionState(joined.session)
  await deps.attachBinding(joined.session, false)
  deps.flashFooter(`joined relay session as ${joined.member.user_id}`, "info")
}

async function listRelayMembers(deps: CloudCommandHandlerDeps): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("attach to a session before listing relay members", "error")
    return
  }
  if (!deps.listCloudSessionMembers) {
    deps.flashFooter("relay member listing is unavailable in this build", "error")
    return
  }
  const listed = await deps.listCloudSessionMembers(deps.sessionState().id)
  const members = (listed.members as Array<{ user_id: string }> | undefined) ?? []
  deps.appendNotice(members.length === 0
    ? "No relay members in this session."
    : members.map((member) => member.user_id).join("\n"))
  deps.flashFooter(`listed ${members.length} relay member${members.length === 1 ? "" : "s"}`, "info")
}

function parsePositiveInt(value: string | undefined): number | null | undefined {
  if (value === undefined) return undefined
  const parsed = Number.parseInt(value, 10)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null
}
