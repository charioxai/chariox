import type { RuntimeSession } from "./cli-types.js"
import {
  formatCloudRelayPendingNotice,
  isRefreshableCloudLinkError,
  startHostedCloudLink,
  waitForHostedCloudRelayConnection,
  type CloudCommandLifecycleDeps,
} from "./cloud-command-lifecycle.js"
import type { RelayCloudProfile } from "./preferences.js"

export type RelayCloudCommandHandlerDeps = CloudCommandLifecycleDeps & {
  sessionState: () => RuntimeSession
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

export async function handleRelayCloudCommand(
  deps: RelayCloudCommandHandlerDeps,
  args: string[],
): Promise<void> {
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
  deps: RelayCloudCommandHandlerDeps,
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

async function pairRelayCloudClient(deps: RelayCloudCommandHandlerDeps, cloudArgs: string[]): Promise<void> {
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
    deps.clientId ?? "chariox-cli",
    alias,
  )
  await deps.saveCloudRelayProfile(paired)
  deps.appendNotice(`cloud client linked: ${paired.clientId ?? deps.clientId}`)
}

async function pairRelayCloudMachine(deps: RelayCloudCommandHandlerDeps, cloudArgs: string[]): Promise<void> {
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

async function connectRelayCloud(deps: RelayCloudCommandHandlerDeps): Promise<void> {
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

async function issueRelayCloudClientToken(deps: RelayCloudCommandHandlerDeps, cloudArgs: string[]): Promise<void> {
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
      ? await deps.pairCloudRelayClient(profile, deps.clientId ?? "chariox-cli", undefined)
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
      `command=chariox --relay-url ${issued.relayUrl} --relay-token ${issued.relayToken} --target-daemon-alias ${targetDaemonAlias}`,
    ].join("\n"),
  )
  deps.appendNotice(`cloud client token minted for ${targetDaemonAlias}`)
}

async function logoutRelayCloud(deps: RelayCloudCommandHandlerDeps, cloudArgs: string[]): Promise<void> {
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
