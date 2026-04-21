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
  userId?: string
  clientId?: string
  machineId?: string
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
    accountId: input.profile.accountId,
    subject: input.subject,
    subjectKind: input.subjectKind,
    realmId: input.profile.realmId,
    userId: input.userId ?? input.profile.userId,
    allowedTargets: input.allowedTargets,
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
