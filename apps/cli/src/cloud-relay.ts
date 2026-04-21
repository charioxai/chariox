import type { RelayCloudProfile } from "./preferences.js"

export type BootstrapCloudRelayInput = {
  apiUrl: string
  email: string
  accountSlug?: string | undefined
}

export type IssueCloudRelayTokenInput = {
  profile: RelayCloudProfile
  subject: string
  subjectKind: "client" | "kernel" | "machine"
  allowedTargets?: string[]
  sessionId?: string
  userId?: string
  clientId?: string
  machineId?: string
}

export type StartCloudDeviceLoginInput = {
  apiUrl: string
  clientId?: string
  clientAlias?: string
  machineId?: string
  machineAlias?: string
}

export type CloudDeviceLoginStart = {
  apiUrl: string
  deviceCode: string
  userCode: string
  verificationUrl: string
  expiresAtMs: number
  intervalSeconds: number
}

export type CloudDeviceLoginPollResult =
  | {
    status: "authorization_pending"
    intervalSeconds: number
    expiresAtMs: number
  }
  | {
    status: "expired_token"
  }
  | {
    status: "approved"
    profile: RelayCloudProfile
  }

export async function bootstrapCloudRelayProfile(
  input: BootstrapCloudRelayInput,
): Promise<RelayCloudProfile> {
  const payload = await postJson<{
    userId: string
    accountId: string
    accountSlug: string
    realmId: string
    relayUrl: string
    issuerId: string
  }>(input.apiUrl, "/account/bootstrap", {
    email: input.email,
    accountSlug: input.accountSlug,
  })
  return {
    apiUrl: normalizeApiUrl(input.apiUrl),
    email: input.email.trim().toLowerCase(),
    accountId: payload.accountId,
    userId: payload.userId,
    accountSlug: payload.accountSlug,
    realmId: payload.realmId,
    relayUrl: payload.relayUrl,
    issuerId: payload.issuerId,
  }
}

export async function startCloudDeviceLogin(
  input: StartCloudDeviceLoginInput,
): Promise<CloudDeviceLoginStart> {
  const payload = await postJson<{
    deviceCode: string
    userCode: string
    verificationUrl: string
    expiresAt: string
    intervalSeconds: number
  }>(input.apiUrl, "/auth/device/start", {
    clientId: input.clientId,
    clientAlias: input.clientAlias,
    machineId: input.machineId,
    machineAlias: input.machineAlias,
  })
  return {
    apiUrl: normalizeApiUrl(input.apiUrl),
    deviceCode: payload.deviceCode,
    userCode: payload.userCode,
    verificationUrl: payload.verificationUrl,
    expiresAtMs: Date.parse(payload.expiresAt),
    intervalSeconds: payload.intervalSeconds,
  }
}

export async function pollCloudDeviceLogin(
  apiUrl: string,
  deviceCode: string,
): Promise<CloudDeviceLoginPollResult> {
  const payload = await postJson<{
    status: "authorization_pending" | "expired_token" | "approved"
    intervalSeconds?: number
    expiresAt?: string
    profile?: RelayCloudProfile & { email: string }
    cloudSessionToken?: string
    cloudSessionExpiresAt?: string
  }>(apiUrl, "/auth/device/poll", { deviceCode })
  if (payload.status === "authorization_pending") {
    return {
      status: "authorization_pending",
      intervalSeconds: payload.intervalSeconds ?? 2,
      expiresAtMs: payload.expiresAt ? Date.parse(payload.expiresAt) : 0,
    }
  }
  if (payload.status === "expired_token") {
    return { status: "expired_token" }
  }
  if (!payload.profile || !payload.cloudSessionToken || !payload.cloudSessionExpiresAt) {
    throw new Error("cloud device login approval response was incomplete")
  }
  return {
    status: "approved",
    profile: {
      ...payload.profile,
      apiUrl: normalizeApiUrl(apiUrl),
      cloudSessionToken: payload.cloudSessionToken,
      cloudSessionExpiresAtMs: Date.parse(payload.cloudSessionExpiresAt),
    },
  }
}

export async function logoutCloudRelayProfile(
  profile: RelayCloudProfile,
  options: { revokeClient?: boolean; revokeMachine?: boolean } = {},
): Promise<void> {
  await postJson(profile.apiUrl, "/auth/logout", {
    sessionToken: profile.cloudSessionToken,
    accountId: profile.accountId,
    clientId: profile.clientId,
    machineId: profile.machineId,
    revokeClient: options.revokeClient,
    revokeMachine: options.revokeMachine,
  })
}

export async function pairCloudRelayClient(
  profile: RelayCloudProfile,
  clientId: string,
  alias?: string,
): Promise<RelayCloudProfile> {
  const pairing = await postJson<{ token: string }>(profile.apiUrl, "/pairing-tokens", {
    accountId: profile.accountId,
    createdByUserId: profile.userId,
    subjectKind: "client",
  })
  await postJson(profile.apiUrl, "/clients/pair", {
    accountId: profile.accountId,
    token: pairing.token,
    clientId,
    userId: profile.userId,
    alias,
  })
  return {
    ...profile,
    clientId,
    ...(alias ? { clientAlias: alias } : {}),
  }
}

export async function pairCloudRelayMachine(
  profile: RelayCloudProfile,
  machineId: string,
  alias?: string,
): Promise<RelayCloudProfile> {
  const pairing = await postJson<{ token: string }>(profile.apiUrl, "/pairing-tokens", {
    accountId: profile.accountId,
    createdByUserId: profile.userId,
    subjectKind: "machine",
  })
  await postJson(profile.apiUrl, "/machines/pair", {
    accountId: profile.accountId,
    token: pairing.token,
    machineId,
    userId: profile.userId,
    ...(alias ? { alias } : {}),
  })
  return {
    ...profile,
    machineId,
    ...(alias ? { machineAlias: alias } : {}),
  }
}

export async function issueCloudRelayToken(
  input: IssueCloudRelayTokenInput,
): Promise<{ relayUrl: string; relayToken: string; tokenExpiresAtMs: number }> {
  const payload = await postJson<{
    token: string
    tokenId: string
    expiresAt: string
  }>(input.profile.apiUrl, "/relay/token", {
    sessionToken: input.profile.cloudSessionToken,
    accountId: input.profile.accountId,
    subject: input.subject,
    subjectKind: input.subjectKind,
    realmId: input.profile.realmId,
    userId: input.userId ?? input.profile.userId,
    allowedTargets: input.allowedTargets,
    sessionId: input.sessionId,
    clientId: input.clientId,
    machineId: input.machineId,
  })
  return {
    relayUrl: input.profile.relayUrl,
    relayToken: payload.token,
    tokenExpiresAtMs: Date.parse(payload.expiresAt),
  }
}

async function postJson<TResponse>(
  apiUrl: string,
  pathname: string,
  body: Record<string, unknown>,
): Promise<TResponse> {
  const response = await fetch(`${normalizeApiUrl(apiUrl)}${pathname}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  })
  return readJson<TResponse>(response)
}

async function readJson<TResponse>(response: Response): Promise<TResponse> {
  const body = await response.json().catch(() => null)
  if (!response.ok) {
    const message = typeof body?.error?.message === "string"
      ? body.error.message
      : `cloud relay request failed with ${response.status}`
    throw new Error(message)
  }
  return body as TResponse
}

function normalizeApiUrl(apiUrl: string): string {
  return apiUrl.trim().replace(/\/+$/, "")
}
