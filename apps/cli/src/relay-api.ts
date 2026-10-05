import type {
  WaitingRoomRelayStatusView,
  WaitingRoomTerminalView,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import type { TerminalPairingLinkJoined } from "@chariox/kernel-client/kernel-types"
import type { RelayCloudProfile } from "./preferences.js"
import { parseAbsoluteInstantMs } from "@chariox/kernel-client/time"
import {
  cloudRelayStatusRequest,
  configureRelayRequest,
  connectCloudRelayRequest,
  createTerminalPairingLinkRequest,
  issueCloudRelayClientTokenRequest,
  joinTerminalPairingLinkRequest,
  logoutCloudRelayRequest,
  pairCloudRelayClientRequest,
  pairCloudRelayMachineRequest,
  pollCloudRelayLoginRequest,
  relayStatusRequest,
  relayClientKeyBindingMinimumProtocolVersion,
  resolveKernelClientConnectionRequest,
  startCloudRelayLoginRequest,
} from "./ipc-requests.js"
import { createCliRelayIdentityStore } from "./cli-relay-identity-store.js"
import { expectVariant } from "./ipc-response.js"
import { sendWithProtocolMinimum } from "./protocol-minimum-diagnostic.js"

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
  kernel_id?: string | null
  kernel_enrolled?: boolean
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

export type KernelClientConnectionView = {
  relayUrl: string
  relayToken: string
  targetDaemonId?: string | null
  targetDaemonAlias?: string | null
  tokenExpiresAtMs?: number | null
  machineId?: string | null
  kernelId?: string | null
}

type KernelClientConnectionPayload = {
  relay_url: string
  relay_token: string
  target_daemon_id?: string | null
  target_daemon_alias?: string | null
  token_expires_at?: string | null
  machine_id?: string | null
  kernel_id?: string | null
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
    expiresAtMs: parseAbsoluteInstantMs(payload.expires_at),
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
      expiresAtMs: payload.expires_at ? parseAbsoluteInstantMs(payload.expires_at) : 0,
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

export async function getKernelCloudRelayProfile(client: LocalIpcClient): Promise<RelayCloudProfile | null> {
  const response = await client.send<Record<string, unknown>>(cloudRelayStatusRequest())
  const payload = expectVariant<{ profile?: KernelCloudRelayProfile | null }>(response, "CloudRelayStatus")
  return payload.profile ? relayCloudProfileFromKernel(payload.profile) : null
}
export async function connectKernelCloudRelay(client: LocalIpcClient) {
  const response = await client.send<Record<string, unknown>>(connectCloudRelayRequest())
  const payload = expectVariant<{ profile: KernelCloudRelayProfile; status: RelayStatusView }>(response, "CloudRelayConnected")
  return { profile: relayCloudProfileFromKernel(payload.profile), status: payload.status }
}

export async function issueKernelCloudRelayClientToken(
  client: LocalIpcClient,
  targetDaemonAlias: string,
  clientId: string,
  sessionId?: string | null,
  publicKeyThumbprint?: string | null,
) {
  const request = issueCloudRelayClientTokenRequest(
    targetDaemonAlias,
    clientId,
    sessionId,
    publicKeyThumbprint,
  )
  const response = publicKeyThumbprint
    ? await sendWithProtocolMinimum<Record<string, unknown>>(
      client.send.bind(client),
      request,
      {
        capability: "CLI key-bound relay tokens",
        requestVariant: "IssueCloudRelayClientToken",
        unknownField: "public_key_thumbprint",
        minimumProtocolVersion: relayClientKeyBindingMinimumProtocolVersion,
      },
    )
    : await client.send<Record<string, unknown>>(request)
  const payload = expectVariant<{
    profile: KernelCloudRelayProfile
    token: KernelCloudRelayRuntimeToken
  }>(response, "CloudRelayClientTokenIssued")
  if (publicKeyThumbprint) {
    requireRelayTokenKeyBinding(payload.token.relay_token, publicKeyThumbprint, "CLI key-bound relay token")
  }
  const targetDaemonId = relayTokenCanonicalTarget(payload.token.relay_token, targetDaemonAlias)
  return {
    relayUrl: payload.token.relay_url,
    relayToken: payload.token.relay_token,
    tokenExpiresAtMs: parseAbsoluteInstantMs(payload.token.token_expires_at),
    ...(targetDaemonId ? { targetDaemonId } : {}),
    profile: relayCloudProfileFromKernel(payload.profile),
  }
}

export async function joinKernelTerminalPairingLink(
  client: LocalIpcClient,
  pairingLink: string,
  terminalId: string | null,
  publicKeyThumbprint: string,
): Promise<TerminalPairingLinkJoined> {
  const response = await sendWithProtocolMinimum<Record<string, unknown>>(
    client.send.bind(client),
    joinTerminalPairingLinkRequest(
      pairingLink,
      terminalId,
      "cli",
      null,
      publicKeyThumbprint,
    ),
    {
      capability: "key-bound terminal pairing",
      requestVariant: "JoinTerminalPairingLink",
      unknownField: "public_key_thumbprint",
      minimumProtocolVersion: relayClientKeyBindingMinimumProtocolVersion,
    },
  )
  const joined = expectVariant<TerminalPairingLinkJoined>(response, "TerminalPairingLinkJoined")
  if (joined.pairing.public_key_thumbprint !== publicKeyThumbprint) {
    throw new Error("terminal pairing response did not confirm this CLI relay identity")
  }
  if (!joined.kernel_pairing && !joined.relay_token?.trim()) {
    throw new Error("key-bound terminal pairing requires a kernel-issued admission or a fresh bound relay token")
  }
  if (joined.relay_token) requireRelayTokenKeyBinding(joined.relay_token, publicKeyThumbprint, "key-bound terminal pairing")
  return joined
}

export function requireRelayTokenKeyBinding(token: string, expectedThumbprint: string, capability: string): void {
  const payload = relayTokenPayload(token)
  if (!payload) {
    throw new Error(`${capability} requires a relay token bound to this CLI's public key`)
  }
  if (payload.public_key_thumbprint !== expectedThumbprint) {
    throw new Error(`${capability} was denied because Cloud relay did not bind the token to this CLI's public key`)
  }
}

function relayTokenCanonicalTarget(token: string, requestedAlias: string): string | undefined {
  // Project launch metadata from the kernel-issued token. This is not an
  // authorization check: the relay verifies the signed token and target scope.
  const targets = relayTokenPayload(token)?.allowed_targets
  if (!Array.isArray(targets) || targets.length !== 1) return undefined
  const target = targets[0]
  return typeof target === "string" && target.trim() && target !== requestedAlias ? target : undefined
}

function relayTokenPayload(token: string): Record<string, unknown> | undefined {
  if (token.length > 16_384) return undefined
  const segments = token.trim().split(".")
  if (segments.length !== 3 || !segments[0] || !segments[1] || !segments[2]) return undefined
  try {
    const payload: unknown = JSON.parse(Buffer.from(segments[1], "base64url").toString("utf8"))
    return payload && typeof payload === "object" && !Array.isArray(payload)
      ? payload as Record<string, unknown> : undefined
  } catch {
    return undefined
  }
}

export async function resolveKernelClientConnection(
  client: LocalIpcClient,
  input: {
    kernelRef: string
    machineRef?: string | null
    clientId?: string | null
    sessionId?: string | null
  },
): Promise<KernelClientConnectionView> {
  const thumbprint = createCliRelayIdentityStore().getOrCreate().publicKeyThumbprint
  const response = await client.send<Record<string, unknown>>(
    resolveKernelClientConnectionRequest({ ...input, publicKeyThumbprint: thumbprint }),
  )
  const payload = expectVariant<{
    connection: KernelClientConnectionPayload
  }>(response, "KernelClientConnectionResolved").connection
  if (payload.token_expires_at) requireRelayTokenKeyBinding(payload.relay_token, thumbprint, "kernel terminal pivot")
  return {
    relayUrl: payload.relay_url,
    relayToken: payload.relay_token,
    targetDaemonId: payload.target_daemon_id ?? null,
    targetDaemonAlias: payload.target_daemon_alias ?? null,
    tokenExpiresAtMs: payload.token_expires_at ? parseAbsoluteInstantMs(payload.token_expires_at) : null,
    machineId: payload.machine_id ?? null,
    kernelId: payload.kernel_id ?? null,
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
    ...(profile.kernel_id ? { kernelId: profile.kernel_id } : {}),
    kernelEnrolled: profile.kernel_enrolled ?? false,
    ...(profile.token_expires_at_ms ? { tokenExpiresAtMs: profile.token_expires_at_ms } : {}),
  }
}
