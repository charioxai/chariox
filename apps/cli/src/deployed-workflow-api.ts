import type { RelayCloudProfile } from "./preferences.js"
import { preparePublicationReleasePackage } from "./deployed-workflow-package.js"
import type {
  AcceptDeploymentClaimResult,
  CreateDeploymentClaimResult,
  CreateDeploymentAudienceApiKeyResult,
  CreateDeploymentAudienceWebhookKeyResult,
  DeploymentAccessResult,
  DeploymentAudienceGrantKind,
  DeploymentAudienceJsonWebKey,
  DeploymentAudienceMode,
  DeploymentAudienceResult,
  DeploymentClaimResult,
  DeploymentControlRole,
  DeploymentCredentialKind,
  DeploymentCredentialCallbackChannelResult,
  DeploymentCredentialEnrollmentResult,
  DeploymentCredentialProfileResult,
  DeploymentCredentialProfilesResult,
  DeploymentEnvironmentDomainsResult,
  DeploymentEnvironmentCredentialsResult,
  DeploymentEnvironmentLimitsResult,
  DeploymentEnvironmentOperationsPolicy,
  DeploymentEnvironmentOperationsResult,
  DeploymentEnvironmentUsageResult,
  DeploymentTelemetryDeletionResult,
  DeploymentTelemetryExportResult,
  DeploymentOwnershipMode,
  DeploymentProjectKind,
  DeploymentEnvironmentResult,
  DeploymentRuntimeLimits,
  DeploymentProjectResult,
  DeploymentProjectsResult,
  PublicationDeploymentMode,
  PublicationReleaseResult,
  ReleasePromotionResult,
  UpsertDeploymentAudienceGrantResult,
} from "./deployed-workflow-types.js"

export async function listDeploymentProjects(
  profile: RelayCloudProfile,
): Promise<DeploymentProjectsResult> {
  return getJson(profile, "/deployment-projects", { accountId: profile.accountId })
}

export async function getDeploymentProject(
  profile: RelayCloudProfile,
  projectId: string,
): Promise<DeploymentProjectResult> {
  return getJson(profile, `/deployment-projects/${encodeURIComponent(projectId)}`, {
    accountId: profile.accountId,
  })
}

export async function createDeploymentProject(
  profile: RelayCloudProfile,
  input: {
    readonly name: string
    readonly slug?: string
    readonly kind: DeploymentProjectKind
    readonly defaultRuntimeMode: PublicationDeploymentMode
    readonly defaultRegion?: string
  },
): Promise<DeploymentProjectResult> {
  return postJson(profile, "/deployment-projects", {
    accountId: profile.accountId,
    name: input.name,
    kind: input.kind,
    defaultRuntimeMode: input.defaultRuntimeMode,
    ...(input.slug ? { slug: input.slug } : {}),
    ...(input.defaultRegion ? { defaultRegion: input.defaultRegion } : {}),
  })
}

export async function adoptLegacyDeploymentProject(
  profile: RelayCloudProfile,
  deploymentId: string,
): Promise<DeploymentProjectResult> {
  return postJson(profile, "/deployment-projects/legacy-adoptions", {
    accountId: profile.accountId,
    deploymentId,
  })
}

export async function createDeploymentRelease(
  profile: RelayCloudProfile,
  projectId: string,
  packagePath: string,
): Promise<PublicationReleaseResult> {
  const prepared = await preparePublicationReleasePackage(packagePath)
  return postJson(
    profile,
    `/deployment-projects/${encodeURIComponent(projectId)}/releases`,
    {
      accountId: profile.accountId,
      ...prepared,
    },
  )
}

export async function promoteDeploymentRelease(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly releaseId: string
    readonly idempotencyKey: string
    readonly configuration?: Record<string, unknown>
    readonly limits?: DeploymentRuntimeLimits
  },
): Promise<ReleasePromotionResult> {
  return postJson(
    profile,
    `/deployment-projects/${encodeURIComponent(input.projectId)}`
      + `/environments/${encodeURIComponent(input.environmentId)}/promotions`,
    {
      accountId: profile.accountId,
      releaseId: input.releaseId,
      idempotencyKey: input.idempotencyKey,
      ...(input.configuration ? { configuration: input.configuration } : {}),
      ...(input.limits ? { limits: input.limits } : {}),
    },
  )
}

export async function rollbackDeploymentEnvironment(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly promotionId: string
    readonly idempotencyKey: string
  },
): Promise<ReleasePromotionResult> {
  return postJson(
    profile,
    `/deployment-projects/${encodeURIComponent(input.projectId)}`
      + `/environments/${encodeURIComponent(input.environmentId)}/rollbacks`,
    {
      accountId: profile.accountId,
      promotionId: input.promotionId,
      idempotencyKey: input.idempotencyKey,
    },
  )
}

export async function changeDeploymentEnvironmentLifecycle(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly action: "start" | "stop" | "restart"
    readonly idempotencyKey: string
  },
): Promise<DeploymentEnvironmentResult> {
  return postJson(
    profile,
    `/deployment-projects/${encodeURIComponent(input.projectId)}`
      + `/environments/${encodeURIComponent(input.environmentId)}/${input.action}`,
    {
      accountId: profile.accountId,
      idempotencyKey: input.idempotencyKey,
    },
  )
}

export async function getDeploymentEnvironmentUsage(
  profile: RelayCloudProfile,
  projectId: string,
  environmentId: string,
): Promise<DeploymentEnvironmentUsageResult> {
  return getJson(profile, `${deploymentEnvironmentPath(projectId, environmentId)}/usage`, {
    accountId: profile.accountId,
  })
}

export async function updateDeploymentEnvironmentLimits(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly limits: DeploymentRuntimeLimits
    readonly idempotencyKey: string
  },
): Promise<DeploymentEnvironmentLimitsResult> {
  return postJson(profile, `${deploymentEnvironmentPath(input.projectId, input.environmentId)}/limits`, {
    accountId: profile.accountId,
    idempotencyKey: input.idempotencyKey,
    limits: input.limits,
  })
}

export async function updateDeploymentEnvironmentOperations(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly policy: DeploymentEnvironmentOperationsPolicy
    readonly idempotencyKey: string
  },
): Promise<DeploymentEnvironmentOperationsResult> {
  return postJson(profile, `${deploymentEnvironmentPath(input.projectId, input.environmentId)}/operations`, {
    accountId: profile.accountId,
    idempotencyKey: input.idempotencyKey,
    policy: input.policy,
  })
}

export async function exportDeploymentEnvironmentTelemetry(
  profile: RelayCloudProfile,
  projectId: string,
  environmentId: string,
): Promise<DeploymentTelemetryExportResult> {
  return postJson(profile, `${deploymentEnvironmentPath(projectId, environmentId)}/telemetry/export`, {
    accountId: profile.accountId,
  })
}

export async function deleteDeploymentEnvironmentTelemetry(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly idempotencyKey: string
  },
): Promise<DeploymentTelemetryDeletionResult> {
  return postJson(profile, `${deploymentEnvironmentPath(input.projectId, input.environmentId)}/telemetry/delete`, {
    accountId: profile.accountId,
    idempotencyKey: input.idempotencyKey,
  })
}

export async function createDeploymentClaim(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly releaseId: string
    readonly ownershipMode: DeploymentOwnershipMode
    readonly builderRole?: DeploymentControlRole | null
    readonly targetAccountId?: string
    readonly targetEmail?: string
    readonly expiresInSeconds?: number
  },
): Promise<CreateDeploymentClaimResult> {
  return postJson(profile, `/deployment-projects/${encodeURIComponent(input.projectId)}/claims`, {
    accountId: profile.accountId,
    releaseId: input.releaseId,
    ownershipMode: input.ownershipMode,
    ...(input.builderRole !== undefined ? { builderRole: input.builderRole } : {}),
    ...(input.targetAccountId ? { targetAccountId: input.targetAccountId } : {}),
    ...(input.targetEmail ? { targetEmail: input.targetEmail } : {}),
    ...(input.expiresInSeconds !== undefined ? { expiresInSeconds: input.expiresInSeconds } : {}),
  })
}

export async function reviewDeploymentClaim(
  profile: RelayCloudProfile,
  claimToken: string,
): Promise<DeploymentClaimResult> {
  return postJson(profile, "/deployment-claims/review", { claimToken })
}

export async function acceptDeploymentClaim(
  profile: RelayCloudProfile,
  input: {
    readonly claimToken: string
    readonly projectName?: string
    readonly projectSlug?: string
    readonly runtimeMode?: PublicationDeploymentMode
  },
): Promise<AcceptDeploymentClaimResult> {
  return postJson(profile, "/deployment-claims/accept", {
    accountId: profile.accountId,
    claimToken: input.claimToken,
    ...(input.projectName ? { projectName: input.projectName } : {}),
    ...(input.projectSlug ? { projectSlug: input.projectSlug } : {}),
    ...(input.runtimeMode ? { runtimeMode: input.runtimeMode } : {}),
  })
}

export async function revokeDeploymentClaim(
  profile: RelayCloudProfile,
  projectId: string,
  claimId: string,
): Promise<DeploymentClaimResult> {
  return postJson(
    profile,
    `/deployment-projects/${encodeURIComponent(projectId)}/claims/${encodeURIComponent(claimId)}/revoke`,
    { accountId: profile.accountId },
  )
}

export async function getDeploymentAccess(
  profile: RelayCloudProfile,
  projectId: string,
): Promise<DeploymentAccessResult> {
  return getJson(profile, `/deployment-projects/${encodeURIComponent(projectId)}/access`, {
    accountId: profile.accountId,
  })
}

export async function upsertDeploymentProjectMember(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly granteeAccountId: string
    readonly userEmail: string
    readonly role: DeploymentControlRole
  },
): Promise<DeploymentAccessResult> {
  return postJson(profile, `/deployment-projects/${encodeURIComponent(input.projectId)}/members`, {
    accountId: profile.accountId,
    granteeAccountId: input.granteeAccountId,
    userEmail: input.userEmail,
    role: input.role,
  })
}

export async function revokeDeploymentProjectMember(
  profile: RelayCloudProfile,
  projectId: string,
  memberId: string,
): Promise<DeploymentAccessResult> {
  return postJson(
    profile,
    `/deployment-projects/${encodeURIComponent(projectId)}/members/${encodeURIComponent(memberId)}/revoke`,
    { accountId: profile.accountId },
  )
}

export async function getDeploymentAudience(
  profile: RelayCloudProfile,
  projectId: string,
  environmentId: string,
): Promise<DeploymentAudienceResult> {
  return getJson(profile, deploymentAudiencePath(projectId, environmentId), {
    accountId: profile.accountId,
  })
}

export async function setDeploymentAudiencePolicy(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly mode: DeploymentAudienceMode
    readonly defaultRoles: readonly string[]
  },
): Promise<DeploymentAudienceResult> {
  return postJson(profile, `${deploymentAudiencePath(input.projectId, input.environmentId)}/policy`, {
    accountId: profile.accountId,
    mode: input.mode,
    defaultRoles: input.defaultRoles,
  })
}

export async function upsertDeploymentAudienceGrant(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly kind: DeploymentAudienceGrantKind
    readonly subject: string
    readonly roles: readonly string[]
    readonly status: "active" | "invited"
    readonly expiresInSeconds?: number | null
  },
): Promise<UpsertDeploymentAudienceGrantResult> {
  return postJson(profile, `${deploymentAudiencePath(input.projectId, input.environmentId)}/grants`, {
    accountId: profile.accountId,
    kind: input.kind,
    subject: input.subject,
    roles: input.roles,
    status: input.status,
    ...(input.expiresInSeconds !== undefined ? { expiresInSeconds: input.expiresInSeconds } : {}),
  })
}

export async function revokeDeploymentAudienceGrant(
  profile: RelayCloudProfile,
  projectId: string,
  environmentId: string,
  grantId: string,
): Promise<DeploymentAudienceResult> {
  return postJson(
    profile,
    `${deploymentAudiencePath(projectId, environmentId)}/grants/${encodeURIComponent(grantId)}/revoke`,
    { accountId: profile.accountId },
  )
}

export async function createDeploymentAudienceApiKey(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly name: string
    readonly roles: readonly string[]
    readonly expiresInSeconds?: number | null
  },
): Promise<CreateDeploymentAudienceApiKeyResult> {
  return postJson(profile, `${deploymentAudiencePath(input.projectId, input.environmentId)}/api-keys`, {
    accountId: profile.accountId,
    name: input.name,
    roles: input.roles,
    ...(input.expiresInSeconds !== undefined ? { expiresInSeconds: input.expiresInSeconds } : {}),
  })
}

export async function revokeDeploymentAudienceApiKey(
  profile: RelayCloudProfile,
  projectId: string,
  environmentId: string,
  apiKeyId: string,
): Promise<DeploymentAudienceResult> {
  return postJson(
    profile,
    `${deploymentAudiencePath(projectId, environmentId)}/api-keys/${encodeURIComponent(apiKeyId)}/revoke`,
    { accountId: profile.accountId },
  )
}

export async function createDeploymentAudienceJwtIssuer(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly name: string
    readonly issuer: string
    readonly audience: string
    readonly jwks: readonly DeploymentAudienceJsonWebKey[]
    readonly roles: readonly string[]
    readonly rolesClaim?: string | null
    readonly expiresInSeconds?: number | null
  },
): Promise<DeploymentAudienceResult> {
  return postJson(profile, `${deploymentAudiencePath(input.projectId, input.environmentId)}/jwt-issuers`, {
    accountId: profile.accountId,
    name: input.name,
    issuer: input.issuer,
    audience: input.audience,
    jwks: input.jwks,
    roles: input.roles,
    ...(input.rolesClaim !== undefined ? { rolesClaim: input.rolesClaim } : {}),
    ...(input.expiresInSeconds !== undefined ? { expiresInSeconds: input.expiresInSeconds } : {}),
  })
}

export async function createDeploymentAudienceWebhookKey(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly name: string
    readonly roles: readonly string[]
    readonly replayWindowSeconds?: number
    readonly expiresInSeconds?: number | null
  },
): Promise<CreateDeploymentAudienceWebhookKeyResult> {
  return postJson(profile, `${deploymentAudiencePath(input.projectId, input.environmentId)}/webhook-keys`, {
    accountId: profile.accountId,
    name: input.name,
    roles: input.roles,
    ...(input.replayWindowSeconds !== undefined ? { replayWindowSeconds: input.replayWindowSeconds } : {}),
    ...(input.expiresInSeconds !== undefined ? { expiresInSeconds: input.expiresInSeconds } : {}),
  })
}

export async function revokeDeploymentAudienceMachineCredential(
  profile: RelayCloudProfile,
  projectId: string,
  environmentId: string,
  credentialId: string,
): Promise<DeploymentAudienceResult> {
  return postJson(
    profile,
    `${deploymentAudiencePath(projectId, environmentId)}/machine-credentials/${encodeURIComponent(credentialId)}/revoke`,
    { accountId: profile.accountId },
  )
}

export async function acceptDeploymentAudienceInvitation(
  profile: RelayCloudProfile,
  grantToken: string,
): Promise<DeploymentAudienceResult> {
  return postJson(profile, "/deployment-audience-invitations/accept", { grantToken })
}

export async function listDeploymentCredentialProfiles(
  profile: RelayCloudProfile,
): Promise<DeploymentCredentialProfilesResult> {
  const result = await getJson<DeploymentCredentialProfilesResult>(
    profile,
    "/deployment-credentials",
    { accountId: profile.accountId },
  )
  return {
    profiles: result.profiles.map(withoutCredentialEnrollmentSetupDetails),
    setupAccess: result.setupAccess === "available" ? "available" : "restricted",
  }
}

export async function getDeploymentCredentialEnrollment(
  profile: RelayCloudProfile,
  profileId: string,
): Promise<DeploymentCredentialEnrollmentResult> {
  return getJson(
    profile,
    `/deployment-credentials/${encodeURIComponent(profileId)}/enrollment`,
    { accountId: profile.accountId },
  )
}

export async function armDeploymentCredentialCallbackChannel(
  profile: RelayCloudProfile,
  input: {
    readonly accountId: string
    readonly enrollmentId: string
    readonly profileId: string
    readonly targetVersion: number
    readonly realmId: string
    readonly kernelTarget: string
    readonly sessionId: string
    readonly agentId: string
  },
): Promise<DeploymentCredentialCallbackChannelResult> {
  if (input.accountId !== profile.accountId) {
    throw new Error("credential callback channel account does not match the linked Cloud profile")
  }
  const { profileId, ...body } = input
  return postJson(
    profile,
    `/deployment-credentials/${encodeURIComponent(profileId)}/enrollment/callback-channel/arm`,
    body,
  )
}

export async function waitForDeploymentCredentialEnrollment(
  profile: RelayCloudProfile,
  profileId: string,
  options: {
    readonly intervalMs?: number
    readonly maxAttempts?: number
  } = {},
): Promise<DeploymentCredentialEnrollmentResult> {
  const intervalMs = Math.max(0, options.intervalMs ?? 1_000)
  const maxAttempts = Math.max(1, options.maxAttempts ?? 6)
  let latest: DeploymentCredentialEnrollmentResult = { enrollment: null }
  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    latest = await getDeploymentCredentialEnrollment(profile, profileId)
    if (credentialEnrollmentActionable(latest)) return latest
    if (attempt + 1 < maxAttempts) await delay(intervalMs)
  }
  return latest
}

export async function createDeploymentCredentialProfile(
  profile: RelayCloudProfile,
  input: {
    readonly kind: DeploymentCredentialKind
    readonly provider?: string
    readonly integration?: string
    readonly label: string
  },
  options: {
    readonly waitForEnrollmentDetails?: boolean
  } = {},
): Promise<DeploymentCredentialProfileResult> {
  const result = await postJson<DeploymentCredentialProfileResult>(profile, "/deployment-credentials", {
    accountId: profile.accountId,
    kind: input.kind,
    ...(input.provider ? { provider: input.provider } : {}),
    ...(input.integration ? { integration: input.integration } : {}),
    label: input.label,
  })
  return options.waitForEnrollmentDetails === false
    ? { ...result, profile: withoutCredentialEnrollmentSetupDetails(result.profile) }
    : withCredentialEnrollmentDetails(profile, result)
}

export async function requestDeploymentCredentialOperation(
  profile: RelayCloudProfile,
  profileId: string,
  operation: "retry" | "test" | "rotate" | "revoke" | "purge",
  options: {
    readonly waitForEnrollmentDetails?: boolean
  } = {},
): Promise<DeploymentCredentialProfileResult> {
  const routeOperation = operation === "retry" ? "setup" : operation
  const result = await postJson<DeploymentCredentialProfileResult>(
    profile,
    `/deployment-credentials/${encodeURIComponent(profileId)}/${routeOperation}`,
    { accountId: profile.accountId },
  )
  if (operation !== "rotate" && operation !== "retry") return result
  return options.waitForEnrollmentDetails === false
    ? { ...result, profile: withoutCredentialEnrollmentSetupDetails(result.profile) }
    : withCredentialEnrollmentDetails(profile, result)
}

export async function getDeploymentEnvironmentCredentials(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly releaseId?: string
  },
): Promise<DeploymentEnvironmentCredentialsResult> {
  return getJson(
    profile,
    `/deployment-projects/${encodeURIComponent(input.projectId)}`
      + `/environments/${encodeURIComponent(input.environmentId)}/credentials`,
    {
      accountId: profile.accountId,
      ...(input.releaseId ? { releaseId: input.releaseId } : {}),
    },
  )
}

export async function bindDeploymentEnvironmentCredential(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly releaseId: string
    readonly slotId: string
    readonly profileId: string
  },
): Promise<DeploymentEnvironmentCredentialsResult> {
  return postJson(
    profile,
    `/deployment-projects/${encodeURIComponent(input.projectId)}`
      + `/environments/${encodeURIComponent(input.environmentId)}/credential-bindings`,
    {
      accountId: profile.accountId,
      releaseId: input.releaseId,
      slotId: input.slotId,
      profileId: input.profileId,
    },
  )
}

export async function revokeDeploymentEnvironmentCredentialBinding(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly slotId: string
  },
): Promise<DeploymentEnvironmentCredentialsResult> {
  return postJson(
    profile,
    `/deployment-projects/${encodeURIComponent(input.projectId)}`
      + `/environments/${encodeURIComponent(input.environmentId)}/credential-bindings/revoke`,
    { accountId: profile.accountId, slotId: input.slotId },
  )
}

async function withCredentialEnrollmentDetails(
  profile: RelayCloudProfile,
  result: DeploymentCredentialProfileResult,
): Promise<DeploymentCredentialProfileResult> {
  const safeProfile = withoutCredentialEnrollmentSetupDetails(result.profile)
  if (!credentialEnrollmentNeedsDetails(safeProfile.enrollment)) return { ...result, profile: safeProfile }
  try {
    const details = await waitForDeploymentCredentialEnrollment(profile, result.profile.id)
    return details.enrollment
      ? {
          ...result,
          profile: { ...safeProfile, enrollment: details.enrollment },
          setupDetailsStatus: "available",
        }
      : { ...result, profile: safeProfile, setupDetailsStatus: "unavailable" }
  } catch {
    return { ...result, profile: safeProfile, setupDetailsStatus: "unavailable" }
  }
}

function credentialEnrollmentActionable(result: DeploymentCredentialEnrollmentResult): boolean {
  const enrollment = result.enrollment
  if (!enrollment) return false
  if (enrollment.mode === "runner_seeded") return true
  if (enrollment.status !== "pending" && enrollment.status !== "claimed") return true
  return Boolean(enrollment.verificationUrl || enrollment.userCode)
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds))
}

function withoutCredentialEnrollmentSetupDetails(
  profile: DeploymentCredentialProfileResult["profile"],
): DeploymentCredentialProfileResult["profile"] {
  if (!profile.enrollment) return profile
  return {
    ...profile,
    enrollment: {
      ...profile.enrollment,
      instructions: credentialEnrollmentNeedsDetails(profile.enrollment)
        ? null
        : profile.enrollment.instructions ?? null,
      verificationUrl: null,
      userCode: null,
    },
  }
}

function credentialEnrollmentNeedsDetails(
  enrollment: DeploymentCredentialProfileResult["profile"]["enrollment"],
): boolean {
  return Boolean(
    enrollment
    && enrollment.mode === "provider_native"
    && (enrollment.status === "pending" || enrollment.status === "claimed"),
  )
}

export async function getDeploymentEnvironmentDomains(
  profile: RelayCloudProfile,
  projectId: string,
  environmentId: string,
): Promise<DeploymentEnvironmentDomainsResult> {
  return getJson(profile, deploymentDomainsPath(projectId, environmentId), {
    accountId: profile.accountId,
  })
}

export async function createDeploymentEnvironmentDomain(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly hostname: string
  },
): Promise<DeploymentEnvironmentDomainsResult> {
  return postJson(profile, deploymentDomainsPath(input.projectId, input.environmentId), {
    accountId: profile.accountId,
    hostname: input.hostname,
  })
}

export async function operateDeploymentEnvironmentDomain(
  profile: RelayCloudProfile,
  input: {
    readonly projectId: string
    readonly environmentId: string
    readonly domainId: string
    readonly operation: "verify" | "canonical" | "remove"
  },
): Promise<DeploymentEnvironmentDomainsResult> {
  return postJson(
    profile,
    `${deploymentDomainsPath(input.projectId, input.environmentId)}`
      + `/${encodeURIComponent(input.domainId)}/${input.operation}`,
    { accountId: profile.accountId },
  )
}

function deploymentDomainsPath(projectId: string, environmentId: string): string {
  return `${deploymentEnvironmentPath(projectId, environmentId)}/domains`
}

function deploymentAudiencePath(projectId: string, environmentId: string): string {
  return `${deploymentEnvironmentPath(projectId, environmentId)}/audience`
}

function deploymentEnvironmentPath(projectId: string, environmentId: string): string {
  return `/deployment-projects/${encodeURIComponent(projectId)}`
    + `/environments/${encodeURIComponent(environmentId)}`
}

async function getJson<TResponse>(
  profile: RelayCloudProfile,
  pathname: string,
  query: Record<string, string>,
): Promise<TResponse> {
  const url = new URL(`${normalizeApiUrl(profile.apiUrl)}${pathname}`)
  for (const [name, value] of Object.entries(query)) url.searchParams.set(name, value)
  return readJson<TResponse>(await fetch(url, { headers: cloudHeaders(profile) }))
}

async function postJson<TResponse>(
  profile: RelayCloudProfile,
  pathname: string,
  body: Record<string, unknown>,
): Promise<TResponse> {
  const response = await fetch(`${normalizeApiUrl(profile.apiUrl)}${pathname}`, {
    method: "POST",
    headers: cloudHeaders(profile),
    body: JSON.stringify(body),
  })
  return readJson<TResponse>(response)
}

async function readJson<TResponse>(response: Response): Promise<TResponse> {
  const body = await response.json().catch(() => null)
  if (!response.ok) {
    const message = typeof body?.error?.message === "string"
      ? body.error.message
      : `deployed workflow request failed with ${response.status}`
    throw new Error(message)
  }
  return body as TResponse
}

function cloudHeaders(profile: RelayCloudProfile): HeadersInit {
  return {
    accept: "application/json",
    "content-type": "application/json",
    ...(profile.cloudSessionToken ? { authorization: `Bearer ${profile.cloudSessionToken}` } : {}),
  }
}

function normalizeApiUrl(apiUrl: string): string {
  return apiUrl.trim().replace(/\/+$/, "")
}
