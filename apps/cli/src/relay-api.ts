import type {
  WaitingRoomRelayStatusView,
  WaitingRoomTerminalView,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import type { RelayCloudProfile } from "./preferences.js"
import {
  configureRelayRequest,
  connectCloudRelayRequest,
  createTerminalPairingLinkRequest,
  issueCloudRelayClientTokenRequest,
  logoutCloudRelayRequest,
  pairCloudRelayClientRequest,
  pairCloudRelayMachineRequest,
  pollCloudRelayLoginRequest,
  relayStatusRequest,
  startCloudRelayLoginRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export type RelayStatusView = WaitingRoomRelayStatusView
export type TerminalTypeView = WaitingRoomTerminalView["terminal_type"]
export type TerminalView = WaitingRoomTerminalView

export type TerminalPairingLinkView = {
  terminal_id: string
  pairing_link: string
  pairing_code: string
  invite_id: string
  relay_url: string
  target_daemon_id: string
  target_daemon_alias?: string | null
  terminal_type: TerminalTypeView
  issued_at_ms: number
  expires_at_ms: number
}

type KernelCloudRelayProfile = {
  api_url: string
  email: string
  account_id: string
  user_id: string
  account_slug: string
  realm_id: string
  relay_url: string
  issuer_id: string
  client_id?: string | null
  client_alias?: string | null
  machine_id?: string | null
  machine_alias?: string | null
  machine_credential?: string | null
  cloud_session_token?: string | null
  cloud_session_expires_at_ms?: number | null
  token_expires_at_ms?: number | null
}

type KernelCloudRelayLoginStart = {
  api_url: string
  device_code: string
  user_code: string
  verification_url: string
  expires_at: string
  interval_seconds: number
}

type KernelCloudRelayLoginPoll = {
  status: "authorization_pending" | "expired_token" | "approved"
  interval_seconds?: number | null
  expires_at?: string | null
  profile?: KernelCloudRelayProfile | null
}

type KernelCloudRelayRuntimeToken = {
  relay_url: string
  relay_token: string
  token_expires_at: string
}

export async function getRelayStatus(client: LocalIpcClient): Promise<RelayStatusView> {
  const response = await client.send<Record<string, unknown>>(relayStatusRequest())
  return expectVariant<{ status: RelayStatusView }>(response, "RelayStatus").status
}

export async function configureRelay(
  client: LocalIpcClient,
  relayUrl: string | null,
  relayToken: string | null,
): Promise<RelayStatusView> {
  const response = await client.send<Record<string, unknown>>(configureRelayRequest(relayUrl, relayToken))
  return expectVariant<{ status: RelayStatusView }>(response, "RelayConfigured").status
}

export async function startCloudRelayLogin(
  client: LocalIpcClient,
  apiUrl: string,
  input: { clientId?: string; machineId?: string; clientAlias?: string; machineAlias?: string },
) {
  const response = await client.send<Record<string, unknown>>(startCloudRelayLoginRequest(apiUrl, input))
  const payload = expectVariant<{ login: KernelCloudRelayLoginStart }>(response, "CloudRelayLoginStarted").login
  return {
    apiUrl: payload.api_url,
    deviceCode: payload.device_code,
    userCode: payload.user_code,
    verificationUrl: payload.verification_url,
    expiresAtMs: Date.parse(payload.expires_at),
    intervalSeconds: payload.interval_seconds,
  }
}

export async function pollCloudRelayLogin(
  client: LocalIpcClient,
  apiUrl: string,
  deviceCode: string,
) {
  const response = await client.send<Record<string, unknown>>(pollCloudRelayLoginRequest(apiUrl, deviceCode))
  const payload = expectVariant<{ result: KernelCloudRelayLoginPoll }>(response, "CloudRelayLoginPolled").result
  if (payload.status === "authorization_pending") {
    return {
      status: "authorization_pending" as const,
      intervalSeconds: payload.interval_seconds ?? 2,
      expiresAtMs: payload.expires_at ? Date.parse(payload.expires_at) : 0,
    }
  }
  if (payload.status === "expired_token") {
    return { status: "expired_token" as const }
  }
  if (!payload.profile) {
    throw new Error("cloud device login approval response was incomplete")
  }
  return {
    status: "approved" as const,
    profile: {
      ...relayCloudProfileFromKernel(payload.profile),
      ...(payload.expires_at ? { cloudSessionExpiresAtMs: Date.parse(payload.expires_at) } : {}),
    },
  }
}

export async function logoutCloudRelay(
  client: LocalIpcClient,
  options: { revokeClient?: boolean; revokeMachine?: boolean } = {},
): Promise<void> {
  const response = await client.send<Record<string, unknown>>(logoutCloudRelayRequest(options))
  expectVariant(response, "CloudRelayLoggedOut")
}

export async function pairKernelCloudRelayClient(
  client: LocalIpcClient,
  clientId: string,
  alias?: string,
): Promise<RelayCloudProfile> {
  const response = await client.send<Record<string, unknown>>(pairCloudRelayClientRequest(clientId, alias))
  const payload = expectVariant<{ profile: KernelCloudRelayProfile }>(response, "CloudRelayClientPaired")
  return relayCloudProfileFromKernel(payload.profile)
}

export async function pairKernelCloudRelayMachine(
  client: LocalIpcClient,
  machineId: string,
  alias?: string,
): Promise<RelayCloudProfile> {
  const response = await client.send<Record<string, unknown>>(pairCloudRelayMachineRequest(machineId, alias))
  const payload = expectVariant<{ profile: KernelCloudRelayProfile }>(response, "CloudRelayMachinePaired")
  return relayCloudProfileFromKernel(payload.profile)
}

export async function connectKernelCloudRelay(client: LocalIpcClient) {
  const response = await client.send<Record<string, unknown>>(connectCloudRelayRequest())
  const payload = expectVariant<{
    profile: KernelCloudRelayProfile
    token: KernelCloudRelayRuntimeToken
  }>(response, "CloudRelayConnected")
  return {
    relayUrl: payload.token.relay_url,
    relayToken: payload.token.relay_token,
    tokenExpiresAtMs: Date.parse(payload.token.token_expires_at),
    profile: relayCloudProfileFromKernel(payload.profile),
  }
}

export async function issueKernelCloudRelayClientToken(
  client: LocalIpcClient,
  targetDaemonAlias: string,
  clientId: string,
  sessionId?: string | null,
) {
  const response = await client.send<Record<string, unknown>>(
    issueCloudRelayClientTokenRequest(targetDaemonAlias, clientId, sessionId),
  )
  const payload = expectVariant<{
    profile: KernelCloudRelayProfile
    token: KernelCloudRelayRuntimeToken
  }>(response, "CloudRelayClientTokenIssued")
  return {
    relayUrl: payload.token.relay_url,
    relayToken: payload.token.relay_token,
    tokenExpiresAtMs: Date.parse(payload.token.token_expires_at),
    profile: relayCloudProfileFromKernel(payload.profile),
  }
}

export async function createTerminalPairingLink(
  client: LocalIpcClient,
  terminalType: TerminalTypeView = "cli",
): Promise<TerminalPairingLinkView> {
  const response = await client.send<Record<string, unknown>>(
    createTerminalPairingLinkRequest(terminalType),
  )
  return expectVariant<{ pairing: TerminalPairingLinkView }>(response, "TerminalPairingLinkCreated").pairing
}

export function formatTerminalTypeLabel(value: TerminalTypeView) {
  switch (value) {
    case "web":
      return "Web terminal"
    case "ios":
      return "iOS terminal"
    case "android":
      return "Android terminal"
    case "cli":
    default:
      return "CLI"
  }
}

export function formatPairingExpiry(expiresAtMs: number) {
  return new Date(expiresAtMs).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  })
}

export function wrapPairingLink(value: string, width: number) {
  const normalizedWidth = Math.max(24, width)
  const lines: string[] = []
  for (let index = 0; index < value.length; index += normalizedWidth) {
    lines.push(value.slice(index, index + normalizedWidth))
  }
  return lines.length > 0 ? lines : [value]
}

export async function renderTerminalPairingQr(pairingLink: string) {
  try {
    const qrcode = await import("qrcode-terminal")
    let output = ""
    qrcode.generate(pairingLink, { small: true }, (qr) => {
      output = qr
    })
    return output.split("\n").filter((line) => line.trim().length > 0)
  } catch {
    return []
  }
}

function relayCloudProfileFromKernel(profile: KernelCloudRelayProfile): RelayCloudProfile {
  return {
    apiUrl: profile.api_url,
    email: profile.email,
    accountId: profile.account_id,
    userId: profile.user_id,
    accountSlug: profile.account_slug,
    realmId: profile.realm_id,
    relayUrl: profile.relay_url,
    issuerId: profile.issuer_id,
    ...(profile.client_id ? { clientId: profile.client_id } : {}),
    ...(profile.client_alias ? { clientAlias: profile.client_alias } : {}),
    ...(profile.machine_id ? { machineId: profile.machine_id } : {}),
    ...(profile.machine_alias ? { machineAlias: profile.machine_alias } : {}),
    ...(profile.machine_credential ? { machineCredential: profile.machine_credential } : {}),
    ...(profile.cloud_session_token ? { cloudSessionToken: profile.cloud_session_token } : {}),
    ...(profile.cloud_session_expires_at_ms ? { cloudSessionExpiresAtMs: profile.cloud_session_expires_at_ms } : {}),
    ...(profile.token_expires_at_ms ? { tokenExpiresAtMs: profile.token_expires_at_ms } : {}),
  }
}
