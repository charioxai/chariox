import type { RelayCloudProfile } from "./preferences.js"
import { preparePublicationReleasePackage } from "./deployed-workflow-package.js"
import type {
  DeploymentProjectKind,
  DeploymentEnvironmentResult,
  DeploymentProjectResult,
  DeploymentProjectsResult,
  PublicationDeploymentMode,
  PublicationReleaseResult,
  ReleasePromotionResult,
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
    readonly limits?: Record<string, unknown>
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
