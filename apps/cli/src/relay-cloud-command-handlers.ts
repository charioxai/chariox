import type { RuntimeSession } from "./cli-types.js"
import {
  formatCloudRelayPendingNotice,
  isRefreshableCloudLinkError,
  startHostedCloudLink,
  waitForHostedCloudRelayConnection,
  type CloudCommandLifecycleDeps,
} from "./cloud-command-lifecycle.js"
import type { RelayCloudProfile } from "./preferences.js"
import { shellQuote } from "./native-tui/launch-environment.js"

export type RelayCloudCommandHandlerDeps = CloudCommandLifecycleDeps & {
  sessionState: () => RuntimeSession
  pairCloudRelayClient?: (
    profile: RelayCloudProfile,
    clientId: string,
    alias?: string,
  ) => Promise<RelayCloudProfile>
  issueCloudClientRelayToken?: (
    profile: RelayCloudProfile,
    targetDaemonAlias: string,
    options?: { sessionId?: string | null },
  ) => Promise<{ relayUrl: string; relayToken: string; tokenExpiresAtMs: number; targetDaemonId?: string; sessionId?: string; profile?: RelayCloudProfile; issuer?: { endpoint: string; daemonId: string } }>
  logoutCloudRelay?: (profile: RelayCloudProfile, options?: { revokeClient?: boolean; revokeMachine?: boolean }) => Promise<void>
}

export async function handleRelayCloudCommand(
  deps: RelayCloudCommandHandlerDeps,
  args: string[],
): Promise<void> {
  const [cloudCommand, ...cloudArgs] = args
  if (!cloudCommand || cloudCommand === "status") {
    const profile = await deps.getCloudRelayProfile?.() ?? null
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
    deps.flashFooter("/relay cloud pair is retired; use /client invite for key-bound terminal pairing", "error")
    return
  }
  if (cloudCommand === "pair-machine") {
    deps.flashFooter("/relay cloud pair-machine is retired; run /cloud link on the receiving kernel", "error")
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
    "usage: /relay cloud status | /relay cloud login [api-url] | /relay cloud connect | /relay cloud client-token <target-daemon-alias> [session-id] | /relay cloud disable",
    "error",
  )
}

async function loginRelayCloud(
  deps: RelayCloudCommandHandlerDeps,
  cloudCommand: string,
  cloudArgs: string[],
): Promise<void> {
  if (cloudCommand === "bootstrap" || cloudArgs.length > 1) {
    deps.flashFooter("email bootstrap is retired; use /cloud link or /relay cloud login [api-url] to enroll this kernel", "error")
    return
  }
  await startHostedCloudLink({...deps, ...(cloudArgs[0] ? {cloudRelayApiUrl: cloudArgs[0]} : {})})
}

async function connectRelayCloud(deps: RelayCloudCommandHandlerDeps): Promise<void> {
  if (deps.connectCloudRelay) {
    const connected = await deps.connectCloudRelay()
    await deps.saveCloudRelayProfile?.(connected.profile)
    const status = await waitForHostedCloudRelayConnection(deps, connected.status)
    await deps.refreshWaitingRoomData?.()
    deps.appendNotice(status?.connected ? `cloud kernel connected: ${connected.profile.relayUrl}` : formatCloudRelayPendingNotice(status, connected.profile.relayUrl))
    return
  }
  if (!deps.issueCloudKernelRelayToken || !deps.saveCloudRelayProfile || !deps.getRelayStatus || !deps.configureRelay) {
    deps.flashFooter("cloud relay connect is unavailable in this build", "error")
    return
  }
  const profile = await deps.getCloudRelayProfile?.() ?? null
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

async function issueRelayCloudClientToken(deps: RelayCloudCommandHandlerDeps, cloudArgs: string[]): Promise<void> {
  if (!deps.issueCloudClientRelayToken || !deps.saveCloudRelayProfile) {
    deps.flashFooter("cloud relay client tokens are unavailable in this build", "error")
    return
  }
  const profile = await deps.getCloudRelayProfile?.() ?? null
  if (!profile) {
    deps.appendNotice("cloud is not linked. Run /cloud first.")
    return
  }
  const targetDaemonAlias = cloudArgs[0]
  if (!targetDaemonAlias) {
    deps.flashFooter("usage: /relay cloud client-token <target-daemon-alias> [session-id]", "error")
    return
  }
  const sessionId = cloudArgs[1] ?? deps.sessionState().id ?? null
  const issued = await deps.issueCloudClientRelayToken(profile, targetDaemonAlias, { sessionId })
  if (issued.profile && deps.saveCloudRelayProfile) {
    await deps.saveCloudRelayProfile(issued.profile)
  }
  deps.appendNotice(
    [
      "cloud client token",
      `transport=${issued.relayUrl}`,
      `expires_at_ms=${issued.tokenExpiresAtMs}`,
      ...(issued.targetDaemonId ? [`target_daemon_id=${issued.targetDaemonId}`] : []),
      ...(issued.issuer ? [`issuer_endpoint=${issued.issuer.endpoint}`, `issuer_daemon_id=${issued.issuer.daemonId}`] : []),
      ...(sessionId ? [`session_id=${sessionId}`] : []),
      "next=select the target kernel in the waiting room to attach with this CLI key",
    ].join("\n"),
  )
  deps.appendNotice(`cloud client token minted for ${targetDaemonAlias}`)
}

async function logoutRelayCloud(deps: RelayCloudCommandHandlerDeps, cloudArgs: string[]): Promise<void> {
  const options = {
    revokeClient: cloudArgs.includes("--revoke-client"),
    revokeMachine: cloudArgs.includes("--revoke-machine"),
  }
  const profile = await deps.getCloudRelayProfile?.() ?? null
  const requiresAcknowledgement = Boolean(profile?.kernelEnrolled) || options.revokeClient || options.revokeMachine
  if (!deps.saveCloudRelayProfile) {
    if (requiresAcknowledgement) throw new Error("cloud relay profile storage is unavailable in this build")
    deps.flashFooter("cloud relay profile storage is unavailable in this build", "error")
    return
  }
  if (requiresAcknowledgement) {
    if (!profile || !deps.logoutCloudRelay) {
      throw new Error("cloud remote revocation requires a linked profile and logout support")
    }
    await deps.logoutCloudRelay(profile, options)
  } else if (profile && deps.logoutCloudRelay) {
    await deps.logoutCloudRelay(profile, options).catch((error) => {
      deps.appendNotice(`cloud logout remote revocation failed: ${deps.formatError(error)}`)
    })
  }
  await deps.saveCloudRelayProfile(null)
  deps.appendNotice("cloud link cleared")
}
