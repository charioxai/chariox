import { readFile } from "node:fs/promises"

import {
  acceptDeploymentAudienceInvitation,
  createDeploymentAudienceApiKey,
  createDeploymentAudienceJwtIssuer,
  createDeploymentAudienceWebhookKey,
  getDeploymentAudience,
  revokeDeploymentAudienceApiKey,
  revokeDeploymentAudienceGrant,
  revokeDeploymentAudienceMachineCredential,
  setDeploymentAudiencePolicy,
  upsertDeploymentAudienceGrant,
} from "./deployed-workflow-api.js"
import type {
  DeploymentAudienceGrantKind,
  DeploymentAudienceJsonWebKey,
  DeploymentAudienceMode,
  DeploymentAudiencePolicySummary,
} from "./deployed-workflow-types.js"
import type { RelayCloudProfile } from "./preferences.js"

export interface DeploymentAudienceCommandOutput {
  readonly notice: string
  readonly footer: string
}

export const deploymentAudienceUsage = [
  "usage: deployments audience show <project-id> <environment-id>",
  "       deployments audience policy <project-id> <environment-id> <public|restricted> [--roles role,...]",
  "       deployments audience grant add <project-id> <environment-id> <email|email-domain|account> <subject> --roles role,... [--status invited|active] [--expires-seconds value|--no-expiry]",
  "       deployments audience grant revoke <project-id> <environment-id> <grant-id>",
  "       deployments audience invite accept <invitation-token>",
  "       deployments audience key create <project-id> <environment-id> <name> --roles role,... [--expires-seconds value|--no-expiry]",
  "       deployments audience key revoke <project-id> <environment-id> <api-key-id>",
  "       deployments audience jwt create <project-id> <environment-id> <name> --issuer <issuer> --audience <audience> --jwks-file <path>|--jwks-json <json> --roles role,... [--roles-claim claim] [--expires-seconds value|--no-expiry]",
  "       deployments audience jwt revoke <project-id> <environment-id> <credential-id>",
  "       deployments audience webhook create <project-id> <environment-id> <name> --roles role,... [--replay-seconds 30..900] [--expires-seconds value|--no-expiry]",
  "       deployments audience webhook revoke <project-id> <environment-id> <credential-id>",
].join("\n")

export async function executeDeploymentAudienceCommand(
  profile: RelayCloudProfile,
  argv: readonly string[],
): Promise<DeploymentAudienceCommandOutput> {
  const action = argv[0] ?? "show"
  if (action === "show" || action === "status") {
    const projectId = requiredArg(argv[1], deploymentAudienceUsage)
    const environmentId = requiredArg(argv[2], deploymentAudienceUsage)
    const result = await getDeploymentAudience(profile, projectId, environmentId)
    return audienceOutput(result.audience)
  }
  if (action === "policy") {
    const projectId = requiredArg(argv[1], deploymentAudienceUsage)
    const environmentId = requiredArg(argv[2], deploymentAudienceUsage)
    const mode = parseMode(requiredArg(argv[3], deploymentAudienceUsage))
    const roles = parsePolicyRoles(argv.slice(4))
    if (mode === "public" && roles.length === 0) {
      throw new Error("public deployment audience requires --roles")
    }
    const result = await setDeploymentAudiencePolicy(profile, {
      projectId,
      environmentId,
      mode,
      defaultRoles: mode === "public" ? roles : [],
    })
    return {
      notice: formatDeploymentAudience(result.audience),
      footer: `deployment audience ${mode}`,
    }
  }
  if (action === "grant") {
    return executeAudienceGrantCommand(profile, argv.slice(1))
  }
  if (action === "key" || action === "api-key" || action === "api_key") {
    return executeAudienceApiKeyCommand(profile, argv.slice(1))
  }
  if (action === "jwt" || action === "jwt-issuer" || action === "jwt_issuer") {
    return executeAudienceJwtCommand(profile, argv.slice(1))
  }
  if (action === "webhook" || action === "webhook-key" || action === "webhook_key") {
    return executeAudienceWebhookCommand(profile, argv.slice(1))
  }
  if ((action === "invite" && argv[1] === "accept") || action === "accept") {
    const token = requiredArg(action === "accept" ? argv[1] : argv[2], deploymentAudienceUsage)
    const result = await acceptDeploymentAudienceInvitation(profile, token)
    return {
      notice: formatDeploymentAudience(result.audience),
      footer: "deployment audience invitation accepted",
    }
  }
  throw new Error(deploymentAudienceUsage)
}

export function formatDeploymentAudience(audience: DeploymentAudiencePolicySummary): string {
  return [
    `audience project=${audience.projectId} environment=${audience.environmentId}`,
    `mode ${audience.mode}`,
    `default_roles ${audience.defaultRoles.join(",") || "none"}`,
    ...audience.routes.map((route) => [
      `route ${route.id} ${route.transport ?? "http"}`,
      `  path ${route.path ?? "none"}`,
      `  required_roles ${route.requiredRoles.join(",") || "none"}`,
    ].join("\n")),
    ...audience.grants.map((grant) => [
      `grant ${grant.id} ${grant.status}`,
      `  identity ${grant.kind} ${grant.subject}`,
      `  roles ${grant.roles.join(",") || "none"}`,
      `  expires_at ${grant.expiresAt ?? "none"}`,
      `  accepted_by ${grant.acceptedByUserId ?? "none"}`,
    ].join("\n")),
    ...audience.apiKeys.map((apiKey) => [
      `api_key ${apiKey.id} ${apiKey.revokedAt ? "revoked" : "active"}`,
      `  name ${apiKey.name}`,
      `  prefix ${apiKey.keyPrefix}`,
      `  roles ${apiKey.roles.join(",") || "none"}`,
      `  expires_at ${apiKey.expiresAt ?? "none"}`,
      `  last_used_at ${apiKey.lastUsedAt ?? "never"}`,
    ].join("\n")),
    ...audience.jwtIssuers.map((issuer) => [
      `jwt_issuer ${issuer.id} ${issuer.revokedAt ? "revoked" : "active"}`,
      `  name ${issuer.name}`,
      `  issuer ${issuer.issuer}`,
      `  audience ${issuer.audience}`,
      `  jwk_key_ids ${issuer.jwkKeyIds.join(",") || "none"}`,
      `  roles ${issuer.roles.join(",") || "none"}`,
      `  roles_claim ${issuer.rolesClaim ?? "none"}`,
      `  expires_at ${issuer.expiresAt ?? "none"}`,
      `  last_used_at ${issuer.lastUsedAt ?? "never"}`,
    ].join("\n")),
    ...audience.webhookKeys.map((webhook) => [
      `webhook_key ${webhook.id} ${webhook.revokedAt ? "revoked" : "active"}`,
      `  name ${webhook.name}`,
      `  key_id ${webhook.keyId}`,
      `  roles ${webhook.roles.join(",") || "none"}`,
      `  replay_window_seconds ${webhook.replayWindowSeconds}`,
      `  expires_at ${webhook.expiresAt ?? "none"}`,
      `  last_used_at ${webhook.lastUsedAt ?? "never"}`,
    ].join("\n")),
  ].join("\n")
}

async function executeAudienceGrantCommand(
  profile: RelayCloudProfile,
  argv: readonly string[],
): Promise<DeploymentAudienceCommandOutput> {
  const action = argv[0]
  if (action === "add" || action === "set") {
    const projectId = requiredArg(argv[1], deploymentAudienceUsage)
    const environmentId = requiredArg(argv[2], deploymentAudienceUsage)
    const kind = parseGrantKind(requiredArg(argv[3], deploymentAudienceUsage))
    const subject = requiredArg(argv[4], deploymentAudienceUsage)
    const options = parseAudienceOptions(argv.slice(5), deploymentAudienceUsage, true)
    if (options.roles.length === 0) throw new Error("deployment audience grant requires --roles")
    if (options.status === "invited" && options.expiresInSeconds === null) {
      throw new Error("deployment audience invitations must expire")
    }
    const result = await upsertDeploymentAudienceGrant(profile, {
      projectId,
      environmentId,
      kind,
      subject,
      roles: options.roles,
      status: options.status,
      ...(options.expiresInSeconds !== undefined ? { expiresInSeconds: options.expiresInSeconds } : {}),
    })
    return {
      notice: [
        formatDeploymentAudience(result.audience),
        ...(result.grantToken ? [`invitation_token ${result.grantToken}`] : []),
      ].join("\n"),
      footer: result.grantToken
        ? "deployment audience invitation created; token shown once"
        : "deployment audience grant active",
    }
  }
  if (action === "revoke") {
    const projectId = requiredArg(argv[1], deploymentAudienceUsage)
    const environmentId = requiredArg(argv[2], deploymentAudienceUsage)
    const grantId = requiredArg(argv[3], deploymentAudienceUsage)
    const result = await revokeDeploymentAudienceGrant(profile, projectId, environmentId, grantId)
    return {
      notice: formatDeploymentAudience(result.audience),
      footer: "deployment audience grant revoked",
    }
  }
  throw new Error(deploymentAudienceUsage)
}

async function executeAudienceApiKeyCommand(
  profile: RelayCloudProfile,
  argv: readonly string[],
): Promise<DeploymentAudienceCommandOutput> {
  const action = argv[0]
  if (action === "create") {
    const projectId = requiredArg(argv[1], deploymentAudienceUsage)
    const environmentId = requiredArg(argv[2], deploymentAudienceUsage)
    const name = requiredArg(argv[3], deploymentAudienceUsage)
    const options = parseAudienceOptions(argv.slice(4), deploymentAudienceUsage, false)
    if (options.roles.length === 0) throw new Error("deployment audience API key requires --roles")
    const result = await createDeploymentAudienceApiKey(profile, {
      projectId,
      environmentId,
      name,
      roles: options.roles,
      ...(options.expiresInSeconds !== undefined ? { expiresInSeconds: options.expiresInSeconds } : {}),
    })
    return {
      notice: `${formatDeploymentAudience(result.audience)}\napi_key_secret ${result.apiKey}`,
      footer: "deployment audience API key created; secret shown once",
    }
  }
  if (action === "revoke") {
    const projectId = requiredArg(argv[1], deploymentAudienceUsage)
    const environmentId = requiredArg(argv[2], deploymentAudienceUsage)
    const apiKeyId = requiredArg(argv[3], deploymentAudienceUsage)
    const result = await revokeDeploymentAudienceApiKey(profile, projectId, environmentId, apiKeyId)
    return {
      notice: formatDeploymentAudience(result.audience),
      footer: "deployment audience API key revoked",
    }
  }
  throw new Error(deploymentAudienceUsage)
}

async function executeAudienceJwtCommand(
  profile: RelayCloudProfile,
  argv: readonly string[],
): Promise<DeploymentAudienceCommandOutput> {
  const action = argv[0]
  if (action === "create") {
    const projectId = requiredArg(argv[1], deploymentAudienceUsage)
    const environmentId = requiredArg(argv[2], deploymentAudienceUsage)
    const name = requiredArg(argv[3], deploymentAudienceUsage)
    const options = await parseJwtOptions(argv.slice(4))
    const result = await createDeploymentAudienceJwtIssuer(profile, {
      projectId,
      environmentId,
      name,
      issuer: options.issuer,
      audience: options.audience,
      jwks: options.jwks,
      roles: options.roles,
      ...(options.rolesClaim !== undefined ? { rolesClaim: options.rolesClaim } : {}),
      ...(options.expiresInSeconds !== undefined ? { expiresInSeconds: options.expiresInSeconds } : {}),
    })
    return {
      notice: formatDeploymentAudience(result.audience),
      footer: "deployment audience JWT issuer created",
    }
  }
  if (action === "revoke") {
    return revokeMachineCredential(profile, argv, "JWT issuer")
  }
  throw new Error(deploymentAudienceUsage)
}

async function executeAudienceWebhookCommand(
  profile: RelayCloudProfile,
  argv: readonly string[],
): Promise<DeploymentAudienceCommandOutput> {
  const action = argv[0]
  if (action === "create") {
    const projectId = requiredArg(argv[1], deploymentAudienceUsage)
    const environmentId = requiredArg(argv[2], deploymentAudienceUsage)
    const name = requiredArg(argv[3], deploymentAudienceUsage)
    const options = parseWebhookOptions(argv.slice(4))
    const result = await createDeploymentAudienceWebhookKey(profile, {
      projectId,
      environmentId,
      name,
      roles: options.roles,
      ...(options.replayWindowSeconds !== undefined ? { replayWindowSeconds: options.replayWindowSeconds } : {}),
      ...(options.expiresInSeconds !== undefined ? { expiresInSeconds: options.expiresInSeconds } : {}),
    })
    return {
      notice: `${formatDeploymentAudience(result.audience)}\nwebhook_secret ${result.webhookSecret}`,
      footer: "deployment audience webhook key created; secret shown once",
    }
  }
  if (action === "revoke") {
    return revokeMachineCredential(profile, argv, "webhook key")
  }
  throw new Error(deploymentAudienceUsage)
}

async function revokeMachineCredential(
  profile: RelayCloudProfile,
  argv: readonly string[],
  label: string,
): Promise<DeploymentAudienceCommandOutput> {
  const projectId = requiredArg(argv[1], deploymentAudienceUsage)
  const environmentId = requiredArg(argv[2], deploymentAudienceUsage)
  const credentialId = requiredArg(argv[3], deploymentAudienceUsage)
  const result = await revokeDeploymentAudienceMachineCredential(profile, projectId, environmentId, credentialId)
  return {
    notice: formatDeploymentAudience(result.audience),
    footer: `deployment audience ${label} revoked`,
  }
}

async function parseJwtOptions(argv: readonly string[]): Promise<{
  readonly issuer: string
  readonly audience: string
  readonly jwks: readonly DeploymentAudienceJsonWebKey[]
  readonly roles: readonly string[]
  readonly rolesClaim?: string | null
  readonly expiresInSeconds?: number | null
}> {
  let issuer: string | undefined
  let audience: string | undefined
  let jwksSource: { readonly kind: "file" | "json"; readonly value: string } | undefined
  let roles: readonly string[] = []
  let rolesClaim: string | null | undefined
  let expiresInSeconds: number | null | undefined
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    if (option === "--issuer") {
      issuer = requiredArg(argv[++index], deploymentAudienceUsage)
    } else if (option === "--audience") {
      audience = requiredArg(argv[++index], deploymentAudienceUsage)
    } else if (option === "--jwks-file" || option === "--jwks-json") {
      if (jwksSource) throw new Error("use one of --jwks-file or --jwks-json")
      jwksSource = {
        kind: option === "--jwks-file" ? "file" : "json",
        value: requiredArg(argv[++index], deploymentAudienceUsage),
      }
    } else if (option === "--roles") {
      roles = parseRoles(requiredArg(argv[++index], deploymentAudienceUsage))
    } else if (option === "--roles-claim") {
      rolesClaim = requiredArg(argv[++index], deploymentAudienceUsage)
    } else if (option === "--expires-seconds") {
      expiresInSeconds = parseExpirySeconds(argv[++index], expiresInSeconds)
    } else if (option === "--no-expiry") {
      if (expiresInSeconds !== undefined) throw new Error(deploymentAudienceUsage)
      expiresInSeconds = null
    } else {
      throw new Error(deploymentAudienceUsage)
    }
  }
  if (!issuer || !audience || !jwksSource || roles.length === 0) throw new Error(deploymentAudienceUsage)
  const jwksText = jwksSource.kind === "file" ? await readFile(jwksSource.value, "utf8") : jwksSource.value
  return {
    issuer,
    audience,
    jwks: parseJwks(jwksText),
    roles,
    ...(rolesClaim !== undefined ? { rolesClaim } : {}),
    ...(expiresInSeconds !== undefined ? { expiresInSeconds } : {}),
  }
}

function parseWebhookOptions(argv: readonly string[]): {
  readonly roles: readonly string[]
  readonly replayWindowSeconds?: number
  readonly expiresInSeconds?: number | null
} {
  let roles: readonly string[] = []
  let replayWindowSeconds: number | undefined
  let expiresInSeconds: number | null | undefined
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    if (option === "--roles") {
      roles = parseRoles(requiredArg(argv[++index], deploymentAudienceUsage))
    } else if (option === "--replay-seconds") {
      const value = Number(requiredArg(argv[++index], deploymentAudienceUsage))
      if (!Number.isSafeInteger(value) || value < 30 || value > 900) {
        throw new Error("--replay-seconds must be an integer between 30 and 900")
      }
      replayWindowSeconds = value
    } else if (option === "--expires-seconds") {
      expiresInSeconds = parseExpirySeconds(argv[++index], expiresInSeconds)
    } else if (option === "--no-expiry") {
      if (expiresInSeconds !== undefined) throw new Error(deploymentAudienceUsage)
      expiresInSeconds = null
    } else {
      throw new Error(deploymentAudienceUsage)
    }
  }
  if (roles.length === 0) throw new Error("deployment audience webhook key requires --roles")
  return {
    roles,
    ...(replayWindowSeconds !== undefined ? { replayWindowSeconds } : {}),
    ...(expiresInSeconds !== undefined ? { expiresInSeconds } : {}),
  }
}

function parseJwks(value: string): readonly DeploymentAudienceJsonWebKey[] {
  let parsed: unknown
  try {
    parsed = JSON.parse(value)
  } catch {
    throw new Error("JWT JWKS must be valid JSON")
  }
  const keys = Array.isArray(parsed)
    ? parsed
    : parsed && typeof parsed === "object" && Array.isArray((parsed as { readonly keys?: unknown }).keys)
      ? (parsed as { readonly keys: readonly unknown[] }).keys
      : null
  if (!keys || keys.length === 0 || keys.length > 10) {
    throw new Error("JWT JWKS must contain between one and ten public keys")
  }
  return keys as readonly DeploymentAudienceJsonWebKey[]
}

function parseExpirySeconds(value: string | undefined, current: number | null | undefined): number {
  if (current !== undefined) throw new Error(deploymentAudienceUsage)
  const parsed = Number(requiredArg(value, deploymentAudienceUsage))
  if (!Number.isSafeInteger(parsed) || parsed < 300 || parsed > 31_536_000) {
    throw new Error("--expires-seconds must be an integer between 300 and 31536000")
  }
  return parsed
}

function parseAudienceOptions(
  argv: readonly string[],
  usage: string,
  allowStatus: boolean,
): {
  readonly roles: readonly string[]
  readonly status: "active" | "invited"
  readonly expiresInSeconds?: number | null
} {
  let roles: readonly string[] = []
  let status: "active" | "invited" = "invited"
  let expiresInSeconds: number | null | undefined
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    if (option === "--roles") {
      roles = parseRoles(requiredArg(argv[index + 1], usage))
      index += 1
    } else if (option === "--status" && allowStatus) {
      const value = requiredArg(argv[index + 1], usage)
      if (value !== "active" && value !== "invited") throw new Error("--status must be active or invited")
      status = value
      index += 1
    } else if (option === "--expires-seconds") {
      if (expiresInSeconds !== undefined) throw new Error(usage)
      const value = Number(requiredArg(argv[index + 1], usage))
      if (!Number.isSafeInteger(value) || value < 300 || value > 31_536_000) {
        throw new Error("--expires-seconds must be an integer between 300 and 31536000")
      }
      expiresInSeconds = value
      index += 1
    } else if (option === "--no-expiry") {
      if (expiresInSeconds !== undefined) throw new Error(usage)
      expiresInSeconds = null
    } else {
      throw new Error(usage)
    }
  }
  return {
    roles,
    status,
    ...(expiresInSeconds !== undefined ? { expiresInSeconds } : {}),
  }
}

function parseRoles(value: string): readonly string[] {
  const roles = [...new Set(value.split(",").map((role) => role.trim().toLowerCase()).filter(Boolean))].sort()
  if (roles.length === 0 || roles.length > 64
    || roles.some((role) => !/^[a-z0-9][a-z0-9:_-]{0,63}$/.test(role))) {
    throw new Error("--roles must contain one to 64 comma-separated deployment roles")
  }
  return roles
}

function parsePolicyRoles(argv: readonly string[]): readonly string[] {
  if (argv.length === 0) return []
  if (argv.length === 2 && argv[0] === "--roles") return parseRoles(requiredArg(argv[1], deploymentAudienceUsage))
  throw new Error(deploymentAudienceUsage)
}

function parseMode(value: string): DeploymentAudienceMode {
  if (value === "public" || value === "restricted") return value
  throw new Error("deployment audience mode must be public or restricted")
}

function parseGrantKind(value: string): DeploymentAudienceGrantKind {
  const normalized = value.replaceAll("-", "_")
  if (normalized === "email" || normalized === "email_domain" || normalized === "account") return normalized
  throw new Error("deployment audience grant kind must be email, email-domain, or account")
}

function audienceOutput(audience: DeploymentAudiencePolicySummary): DeploymentAudienceCommandOutput {
  return {
    notice: formatDeploymentAudience(audience),
    footer: `deployment audience ${audience.mode}`,
  }
}

function requiredArg(value: string | undefined, usage: string): string {
  if (!value?.trim()) throw new Error(usage)
  return value.trim()
}
