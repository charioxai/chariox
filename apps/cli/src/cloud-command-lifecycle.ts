import type { RelayCloudProfile } from "./preferences.js"

export type RelayStatus = {
  configured: boolean
  connected: boolean
  relay_url?: string | null
  relay_token_configured: boolean
  daemon_id: string
  machine_id: string
  machine_alias?: string | null
}

export type CloudCommandLifecycleDeps = {
  clientId?: string | null
  appendNotice: (message: string) => void
  appendCloudNotice?: (message: string) => void
  flashFooter: (message: string, tone: "info" | "error") => void
  formatError: (error: unknown) => string
  cloudRelayApiUrl?: string | undefined
  cloudRelayConnectTimeoutMs?: number
  cloudRelayConnectPollMs?: number
  getCloudRelayProfile?: () => RelayCloudProfile | null
  saveCloudRelayProfile?: (profile: RelayCloudProfile | null) => Promise<void>
  getRelayStatus?: () => Promise<RelayStatus>
  configureRelay?: (relayUrl: string | null, relayToken: string | null) => Promise<RelayStatus>
  refreshWaitingRoomData?: () => Promise<void>
  startCloudDeviceLogin?: (
    apiUrl: string,
    input: { clientId?: string; machineId?: string; clientAlias?: string; machineAlias?: string },
  ) => Promise<{
    apiUrl: string
    deviceCode: string
    userCode: string
    verificationUrl: string
    expiresAtMs: number
    intervalSeconds: number
  }>
  pollCloudDeviceLogin?: (
    apiUrl: string,
    deviceCode: string,
  ) => Promise<
    | { status: "authorization_pending"; intervalSeconds: number; expiresAtMs: number }
    | { status: "expired_token" }
    | { status: "approved"; profile: RelayCloudProfile }
  >
  openExternalUrl?: (url: string) => Promise<boolean>
  pairCloudRelayMachine?: (
    profile: RelayCloudProfile,
    machineId: string,
    alias?: string,
  ) => Promise<RelayCloudProfile>
  issueCloudKernelRelayToken?: (
    profile: RelayCloudProfile,
    daemonId: string,
  ) => Promise<{ relayUrl: string; relayToken: string; tokenExpiresAtMs: number; profile?: RelayCloudProfile }>
  issueCloudMachineRelayToken?: (
    profile: RelayCloudProfile,
    daemonId: string,
    machineId: string,
  ) => Promise<{ relayUrl: string; relayToken: string; tokenExpiresAtMs: number; profile?: RelayCloudProfile }>
}

type HostedCloudRelayEnsureResult = {
  profile: RelayCloudProfile
  status: RelayStatus | null
}

const DEFAULT_HOSTED_CLOUD_API_URL = "https://arroba-cloud-staging.osc-fr1.scalingo.io"
const HOSTED_CLOUD_RELAY_CONNECT_TIMEOUT_MS = 8_000
const HOSTED_CLOUD_RELAY_CONNECT_POLL_MS = 500

export function buildHostedCloudTerminalUrl(apiUrl: string): string {
  const url = new URL("/terminal", apiUrl)
  url.searchParams.set("view", "waiting")
  return url.toString()
}

export function isMissingKernelCloudProfileError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  return message.includes("cloud relay profile missing") || message.includes("run /relay cloud login first")
}

export function isRefreshableCloudLinkError(error: unknown): boolean {
  if (isMissingKernelCloudProfileError(error)) {
    return true
  }
  const message = error instanceof Error ? error.message : String(error)
  return message.includes("identity_revoked")
    || message.includes("Subject has been revoked")
    || message.includes("session_invalid")
    || message.includes("session_expired")
    || message.includes("invalid_session")
    || message.includes("realm_not_found")
    || message.includes("account_deleted")
    || message.includes("user_deleted")
    || message.includes("cloud_api_code=session_invalid")
    || message.includes("cloud relay request failed with 401")
}

export function formatCloudRelayPendingNotice(
  status: RelayStatus | null,
  relayUrl: string | null | undefined,
): string {
  return [
    "cloud machine linked, but the kernel is not online in Cloud yet.",
    `relay=${status?.connected ? "connected" : "not connected"}`,
    `relay_url=${status?.relay_url ?? relayUrl ?? "-"}`,
    `machine=${status?.machine_id ?? "-"}`,
    "next=keep this CLI running; run /cloud link if the link was revoked or /cloud status to check again.",
  ].join("\n")
}

export async function waitForHostedCloudRelayConnection(
  deps: CloudCommandLifecycleDeps,
  status: RelayStatus | null,
): Promise<RelayStatus | null> {
  if (!deps.getRelayStatus || status?.connected) {
    return status
  }
  const timeoutMs = deps.cloudRelayConnectTimeoutMs ?? HOSTED_CLOUD_RELAY_CONNECT_TIMEOUT_MS
  const pollMs = deps.cloudRelayConnectPollMs ?? HOSTED_CLOUD_RELAY_CONNECT_POLL_MS
  const deadline = Date.now() + Math.max(timeoutMs, 0)
  let latest = status
  while (!latest?.connected && Date.now() < deadline) {
    await sleep(Math.max(pollMs, 1))
    latest = await deps.getRelayStatus()
  }
  return latest
}

export async function openHostedCloud(deps: CloudCommandLifecycleDeps): Promise<void> {
  const currentProfile = deps.getCloudRelayProfile?.() ?? null
  if (!currentProfile) {
    await startHostedCloudLink(deps)
    return
  }
  const terminalUrl = buildHostedCloudTerminalUrl(currentProfile.apiUrl)
  let relayLine: string | null = null
  try {
    const ensured = await ensureHostedCloudRelay(deps, currentProfile)
    if (ensured.status && !ensured.status.connected) {
      relayLine = formatCloudRelayPendingNotice(ensured.status, ensured.profile.relayUrl)
    }
  } catch (error) {
    if (isMissingKernelCloudProfileError(error)) {
      await startHostedCloudLink(deps)
      return
    }
    if (isRefreshableCloudLinkError(error)) {
      appendCloudNotice(deps, "Cloud link needs refresh. Starting link flow.")
      await startHostedCloudLink(deps)
      return
    }
    relayLine = [
      "cloud relay could not be refreshed.",
      `error=${deps.formatError(error)}`,
      "next=run /cloud link to refresh pairing.",
    ].join("\n")
  }
  const opened = await deps.openExternalUrl?.(terminalUrl)
  appendCloudNotice(
    deps,
    [
      "Opening Arroba Cloud.",
      `url=${terminalUrl}`,
      opened ? "browser=opened" : "browser=manual",
      ...(relayLine ? [relayLine] : []),
    ].join("\n"),
  )
  deps.flashFooter(opened ? "opened Arroba Cloud" : "Arroba Cloud URL ready", "info")
}

export async function startHostedCloudLink(deps: CloudCommandLifecycleDeps): Promise<void> {
  if (!deps.startCloudDeviceLogin || !deps.pollCloudDeviceLogin || !deps.getRelayStatus || !deps.saveCloudRelayProfile) {
    deps.flashFooter("cloud login is unavailable in this build", "error")
    return
  }
  const relayStatus = await deps.getRelayStatus()
  const started = await deps.startCloudDeviceLogin(deps.cloudRelayApiUrl ?? DEFAULT_HOSTED_CLOUD_API_URL, {
    clientId: deps.clientId ?? "arroba-cli",
    machineId: relayStatus.machine_id,
    ...(relayStatus.machine_alias ? { machineAlias: relayStatus.machine_alias } : {}),
  })
  const opened = await deps.openExternalUrl?.(started.verificationUrl)
  appendCloudNotice(
    deps,
    [
      "Link this machine to Arroba Cloud.",
      `url=${started.verificationUrl}`,
      `code=${started.userCode}`,
      opened ? "browser=opened" : "browser=manual",
    ].join("\n"),
  )
  let intervalMs = Math.max(started.intervalSeconds, 1) * 1000
  while (Date.now() < started.expiresAtMs) {
    const polled = await deps.pollCloudDeviceLogin(started.apiUrl, started.deviceCode)
    if (polled.status === "approved") {
      let profile = polled.profile
      await deps.saveCloudRelayProfile(profile)
      if (deps.pairCloudRelayMachine) {
        profile = await deps.pairCloudRelayMachine(
          profile,
          relayStatus.machine_id,
          relayStatus.machine_alias || undefined,
        )
        await deps.saveCloudRelayProfile(profile)
        appendCloudNotice(deps, `cloud machine linked: ${profile.machineId ?? relayStatus.machine_id}`)
      }
      if ((deps.issueCloudMachineRelayToken || deps.issueCloudKernelRelayToken) && deps.getRelayStatus && deps.configureRelay) {
        const refreshedRelayStatus = await deps.getRelayStatus()
        const issued = profile.machineId && deps.issueCloudMachineRelayToken
          ? await deps.issueCloudMachineRelayToken(profile, refreshedRelayStatus.daemon_id, profile.machineId)
          : await deps.issueCloudKernelRelayToken!(profile, refreshedRelayStatus.daemon_id)
        const configuredStatus = await deps.configureRelay(issued.relayUrl, issued.relayToken)
        profile = {
          ...(issued.profile ?? profile),
          tokenExpiresAtMs: issued.tokenExpiresAtMs,
        }
        await deps.saveCloudRelayProfile(profile)
        const connectedStatus = await waitForHostedCloudRelayConnection(deps, configuredStatus)
        appendCloudNotice(
          deps,
          connectedStatus?.connected
            ? `cloud kernel connected: ${issued.relayUrl}`
            : formatCloudRelayPendingNotice(connectedStatus ?? configuredStatus, issued.relayUrl),
        )
      }
      await deps.refreshWaitingRoomData?.()
      appendCloudNotice(deps, `cloud linked: ${profile.accountSlug}`)
      return
    }
    if (polled.status === "expired_token") {
      appendCloudNotice(deps, "cloud login expired")
      return
    }
    intervalMs = Math.max(polled.intervalSeconds, 1) * 1000
    await sleep(intervalMs)
  }
  appendCloudNotice(deps, "cloud login expired")
}

async function ensureHostedCloudRelay(
  deps: CloudCommandLifecycleDeps,
  profile: RelayCloudProfile,
): Promise<HostedCloudRelayEnsureResult> {
  if (!deps.getRelayStatus || !deps.configureRelay || (!deps.issueCloudMachineRelayToken && !deps.issueCloudKernelRelayToken)) {
    return { profile, status: null }
  }
  const relayStatus = await deps.getRelayStatus()
  if (relayStatus.configured && relayStatus.connected) {
    return { profile, status: relayStatus }
  }
  const issued = profile.machineId && deps.issueCloudMachineRelayToken
    ? await deps.issueCloudMachineRelayToken(profile, relayStatus.daemon_id, profile.machineId)
    : deps.issueCloudKernelRelayToken
      ? await deps.issueCloudKernelRelayToken(profile, relayStatus.daemon_id)
      : null
  if (!issued) {
    return { profile, status: relayStatus }
  }
  const configuredStatus = await deps.configureRelay(issued.relayUrl, issued.relayToken)
  const nextProfile = {
    ...(issued.profile ?? profile),
    tokenExpiresAtMs: issued.tokenExpiresAtMs,
  }
  await deps.saveCloudRelayProfile?.(nextProfile)
  const connectedStatus = await waitForHostedCloudRelayConnection(deps, configuredStatus)
  await deps.refreshWaitingRoomData?.()
  return { profile: nextProfile, status: connectedStatus ?? configuredStatus }
}

function appendCloudNotice(deps: CloudCommandLifecycleDeps, message: string): void {
  ;(deps.appendCloudNotice ?? deps.appendNotice)(message)
}

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
