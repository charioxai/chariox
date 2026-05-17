import type { ParsedSlashCommand } from "./commands.js"
import type { RelayCloudProfile } from "./preferences.js"
import {
  handleCloudSessionCommand,
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
  deps.flashFooter("usage: /relay status | /relay use <ws-url> [token] | /relay disable", "error")
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
  if (await handleCloudSessionCommand(deps, profile, area, action, args)) {
    return
  }
  deps.flashFooter("usage: /cloud [open|link|status] | /cloud invite create [max-uses] | /cloud invite accept <invite-token-or-url> | /cloud members | /cloud collaborators", "error")
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
