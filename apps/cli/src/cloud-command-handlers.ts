import type { ParsedSlashCommand } from "./commands.js"
import type { RelayCloudProfile } from "./preferences.js"
import {
  handleCloudSessionCommand,
  type CloudSessionCommandHandlerDeps,
} from "./cloud-session-command-handlers.js"
import {
  buildHostedCloudTerminalUrl,
  formatCloudRelayPendingNotice,
  isRefreshableCloudLinkError,
  openHostedCloud,
  startHostedCloudLink,
  waitForHostedCloudRelayConnection,
  type CloudCommandLifecycleDeps,
  type RelayStatus,
} from "./cloud-command-lifecycle.js"

export type CloudCommandHandlerDeps = CloudCommandLifecycleDeps & CloudSessionCommandHandlerDeps & {
  bootstrapCloudRelay?: (
    apiUrl: string,
    email: string,
    accountSlug?: string,
  ) => Promise<RelayCloudProfile>
  pairCloudRelayClient?: (
    profile: RelayCloudProfile,
    clientId: string,
    alias?: string,
  ) => Promise<RelayCloudProfile>
  issueCloudClientRelayToken?: (
    profile: RelayCloudProfile,
    targetDaemonAlias: string,
    options?: { sessionId?: string | null },
  ) => Promise<{ relayUrl: string; relayToken: string; tokenExpiresAtMs: number; profile?: RelayCloudProfile }>
  logoutCloudRelay?: (profile: RelayCloudProfile, options?: { revokeClient?: boolean; revokeMachine?: boolean }) => Promise<void>
}

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

async function handleRelayCloudCommand(deps: CloudCommandHandlerDeps, args: string[]): Promise<void> {
  const [cloudCommand, ...cloudArgs] = args
  if (!cloudCommand || cloudCommand === "status") {
    const profile = deps.getCloudRelayProfile?.() ?? null
    if (!profile) {
      deps.appendNotice("cloud is not linked. Run /cloud first.")
      return
    }
    deps.appendNotice(
      [
        "cloud profile",
        `api=${profile.apiUrl}`,
        `email=${profile.email}`,
        `account=${profile.accountSlug} (${profile.accountId})`,
        `cloud=${profile.realmId}`,
        `transport=${profile.relayUrl}`,
        `client=${profile.clientId ?? "-"}`,
        `machine=${profile.machineId ?? "-"}`,
      ].join("\n"),
    )
    return
  }
  if (cloudCommand === "login" || cloudCommand === "bootstrap") {
    await loginRelayCloud(deps, cloudCommand, cloudArgs)
    return
  }
  if (cloudCommand === "pair") {
    await pairRelayCloudClient(deps, cloudArgs)
    return
  }
  if (cloudCommand === "pair-machine") {
    await pairRelayCloudMachine(deps, cloudArgs)
    return
  }
  if (cloudCommand === "connect" || cloudCommand === "refresh") {
    await connectRelayCloud(deps)
    return
  }
  if (cloudCommand === "client-token") {
    await issueRelayCloudClientToken(deps, cloudArgs)
    return
  }
  if (cloudCommand === "disable" || cloudCommand === "logout") {
    await logoutRelayCloud(deps, cloudArgs)
    return
  }
  deps.flashFooter(
    "usage: /relay cloud status | /relay cloud login <api-url> <email> [account-slug] | /relay cloud pair [alias] | /relay cloud pair-machine [machine-id] [alias] | /relay cloud connect | /relay cloud client-token <target-daemon-alias> [session-id] | /relay cloud disable",
    "error",
  )
}

async function loginRelayCloud(
  deps: CloudCommandHandlerDeps,
  cloudCommand: string,
  cloudArgs: string[],
): Promise<void> {
  if (!deps.bootstrapCloudRelay || !deps.saveCloudRelayProfile) {
    deps.flashFooter("cloud relay login is unavailable in this build", "error")
    return
  }
  const apiUrl = cloudArgs[0]
  const email = cloudArgs[1]
  const accountSlug = cloudArgs[2]
  if (!apiUrl && cloudCommand === "login") {
    await startHostedCloudLink(deps)
    return
  }
  if (!apiUrl || !email) {
    deps.flashFooter("usage: /relay cloud login <api-url> <email> [account-slug]", "error")
    return
  }
  const profile = await deps.bootstrapCloudRelay(apiUrl, email, accountSlug)
  await deps.saveCloudRelayProfile(profile)
  deps.appendNotice(`cloud profile saved: ${profile.accountSlug}`)
}

async function pairRelayCloudClient(deps: CloudCommandHandlerDeps, cloudArgs: string[]): Promise<void> {
  if (!deps.pairCloudRelayClient || !deps.saveCloudRelayProfile) {
    deps.flashFooter("cloud relay client pairing is unavailable in this build", "error")
    return
  }
  const profile = deps.getCloudRelayProfile?.() ?? null
  if (!profile) {
    deps.appendNotice("cloud is not linked. Run /cloud first.")
    return
  }
  const alias = cloudArgs.join(" ").trim() || undefined
  const paired = await deps.pairCloudRelayClient(
    profile,
    deps.clientId ?? "arroba-cli",
    alias,
  )
  await deps.saveCloudRelayProfile(paired)
  deps.appendNotice(`cloud client linked: ${paired.clientId ?? deps.clientId}`)
}

async function pairRelayCloudMachine(deps: CloudCommandHandlerDeps, cloudArgs: string[]): Promise<void> {
  if (!deps.pairCloudRelayMachine || !deps.saveCloudRelayProfile || !deps.getRelayStatus) {
    deps.flashFooter("cloud relay machine pairing is unavailable in this build", "error")
    return
  }
  const profile = deps.getCloudRelayProfile?.() ?? null
  if (!profile) {
    deps.appendNotice("cloud is not linked. Run /cloud first.")
    return
  }
  const relayStatus = await deps.getRelayStatus()
  const machineId = cloudArgs[0] || relayStatus.machine_id
  if (!machineId) {
    deps.flashFooter("usage: /relay cloud pair-machine [machine-id] [alias]", "error")
    return
  }
  const alias = cloudArgs.slice(1).join(" ").trim() || relayStatus.machine_alias || undefined
  const paired = await deps.pairCloudRelayMachine(profile, machineId, alias)
  await deps.saveCloudRelayProfile(paired)
  deps.appendNotice(`cloud machine linked: ${paired.machineId ?? machineId}`)
}

async function connectRelayCloud(deps: CloudCommandHandlerDeps): Promise<void> {
  if (!deps.issueCloudKernelRelayToken || !deps.saveCloudRelayProfile || !deps.getRelayStatus || !deps.configureRelay) {
    deps.flashFooter("cloud relay connect is unavailable in this build", "error")
    return
  }
  const profile = deps.getCloudRelayProfile?.() ?? null
  if (!profile) {
    deps.appendNotice("cloud is not linked. Run /cloud first.")
    return
  }
  const relayStatus = await deps.getRelayStatus()
  let issued: { relayUrl: string; relayToken: string; tokenExpiresAtMs: number; profile?: RelayCloudProfile }
  try {
    issued = profile.machineId && deps.issueCloudMachineRelayToken
      ? await deps.issueCloudMachineRelayToken(profile, relayStatus.daemon_id, profile.machineId)
      : await deps.issueCloudKernelRelayToken(profile, relayStatus.daemon_id)
  } catch (error) {
    if (isRefreshableCloudLinkError(error)) {
      deps.appendNotice(
        [
          "cloud link needs refresh.",
          `error=${deps.formatError(error)}`,
          "next=run /cloud link to reconnect this machine.",
        ].join("\n"),
      )
      deps.flashFooter("cloud link needs refresh", "error")
      return
    }
    throw error
  }
  const configuredStatus = await deps.configureRelay(issued.relayUrl, issued.relayToken)
  await deps.saveCloudRelayProfile({
    ...(issued.profile ?? profile),
    tokenExpiresAtMs: issued.tokenExpiresAtMs,
  })
  const connectedStatus = await waitForHostedCloudRelayConnection(deps, configuredStatus)
  await deps.refreshWaitingRoomData?.()
  deps.appendNotice(
    connectedStatus?.connected
      ? `cloud kernel connected: ${issued.relayUrl}`
      : formatCloudRelayPendingNotice(connectedStatus ?? configuredStatus, issued.relayUrl),
  )
}

async function issueRelayCloudClientToken(deps: CloudCommandHandlerDeps, cloudArgs: string[]): Promise<void> {
  if (!deps.issueCloudClientRelayToken || !deps.saveCloudRelayProfile) {
    deps.flashFooter("cloud relay client tokens are unavailable in this build", "error")
    return
  }
  const profile = deps.getCloudRelayProfile?.() ?? null
  if (!profile) {
    deps.appendNotice("cloud is not linked. Run /cloud first.")
    return
  }
  const targetDaemonAlias = cloudArgs[0]
  if (!targetDaemonAlias) {
    deps.flashFooter("usage: /relay cloud client-token <target-daemon-alias> [session-id]", "error")
    return
  }
  const ensuredProfile = profile.clientId
    ? profile
    : deps.pairCloudRelayClient
      ? await deps.pairCloudRelayClient(profile, deps.clientId ?? "arroba-cli", undefined)
      : profile
  if (!profile.clientId && deps.saveCloudRelayProfile) {
    await deps.saveCloudRelayProfile(ensuredProfile)
  }
  const sessionId = cloudArgs[1] ?? deps.sessionState().id ?? null
  const issued = await deps.issueCloudClientRelayToken(ensuredProfile, targetDaemonAlias, { sessionId })
  if (issued.profile && deps.saveCloudRelayProfile) {
    await deps.saveCloudRelayProfile(issued.profile)
  }
  deps.appendNotice(
    [
      "cloud client token",
      `transport=${issued.relayUrl}`,
      `expires_at_ms=${issued.tokenExpiresAtMs}`,
      ...(sessionId ? [`session_id=${sessionId}`] : []),
      `command=arroba --relay-url ${issued.relayUrl} --relay-token ${issued.relayToken} --target-daemon-alias ${targetDaemonAlias}`,
    ].join("\n"),
  )
  deps.appendNotice(`cloud client token minted for ${targetDaemonAlias}`)
}

async function logoutRelayCloud(deps: CloudCommandHandlerDeps, cloudArgs: string[]): Promise<void> {
  if (!deps.saveCloudRelayProfile) {
    deps.flashFooter("cloud relay profile storage is unavailable in this build", "error")
    return
  }
  const profile = deps.getCloudRelayProfile?.() ?? null
  if (profile && deps.logoutCloudRelay) {
    await deps.logoutCloudRelay(profile, {
      revokeClient: cloudArgs.includes("--revoke-client"),
      revokeMachine: cloudArgs.includes("--revoke-machine"),
    }).catch((error) => {
      deps.appendNotice(`cloud logout remote revocation failed: ${deps.formatError(error)}`)
    })
  }
  await deps.saveCloudRelayProfile(null)
  deps.appendNotice("cloud link cleared")
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
