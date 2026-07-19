import type { RelayCloudProfile } from "./preferences.js"

export type DeploymentSetupOrigin = "draft" | "publication"
export type DeploymentSetupStatus = "active" | "blocked" | "completed" | "abandoned"
export type DeploymentSetupStage =
  | "source"
  | "package"
  | "project"
  | "release"
  | "credentials"
  | "runtime"
  | "activation"
  | "complete"

export type DeploymentSetupRuntimeMode = "local_runtime" | "hosted_container"

export interface DeploymentSetupConfiguration {
  readonly endpointId: string
  readonly publication: {
    readonly alias: string
    readonly kind: string
    readonly queueRef?: string | null
    readonly route?: string | null
    readonly methods?: readonly string[]
    readonly transport?: unknown
    readonly parser?: unknown
    readonly inputSchema?: unknown
    readonly traceExposure?: unknown
    readonly mode?: string | null
    readonly syncTimeoutMs?: number | null
    readonly pollMs?: number | null
  }
  readonly deployment: {
    readonly name: string
    readonly slug: string
    readonly kind: "workflow_endpoint" | "agent_app"
    readonly runtimeMode: DeploymentSetupRuntimeMode
    readonly projectId?: string | null
    readonly environmentSlug?: string | null
    readonly region?: string | null
    readonly configuration?: unknown
    readonly configurationDigest?: string | null
    readonly limits?: unknown
  }
  readonly agentApp?: {
    readonly enabled: boolean
    readonly routePath?: string | null
    readonly manipulationLevel?: string | null
    readonly replicaCount?: number | null
  } | null
}

export interface DeploymentSetupOperationKeys {
  readonly publication: string
  readonly package: string
  readonly project: string
  readonly release: string
  readonly credentials: string
  readonly promotion: string
  readonly runtime: string
}

export interface DeploymentSetup {
  readonly id: string
  readonly accountId: string
  readonly createdByUserId: string
  readonly clientRequestId: string
  readonly origin: DeploymentSetupOrigin
  readonly status: DeploymentSetupStatus
  readonly stage: DeploymentSetupStage
  readonly version: number
  readonly sourceSessionId: string
  readonly sourceWorkflowId: string
  readonly sourceWorkflowRevision?: string | null
  readonly sourcePublicationId?: string | null
  readonly sourcePublicationDigest?: string | null
  readonly configuration: DeploymentSetupConfiguration
  readonly packageId?: string | null
  readonly packageDigest?: string | null
  readonly projectId?: string | null
  readonly releaseId?: string | null
  readonly environmentId?: string | null
  readonly promotionId?: string | null
  readonly operationalDeploymentId?: string | null
  readonly failureCode?: string | null
  readonly failureMessage?: string | null
  readonly completedAt?: string | null
  readonly abandonedAt?: string | null
  readonly createdAt: string
  readonly updatedAt: string
  readonly operationKeys: DeploymentSetupOperationKeys
}

export interface CreateDeploymentSetupInput {
  readonly clientRequestId: string
  readonly origin: DeploymentSetupOrigin
  readonly sourceSessionId: string
  readonly sourceWorkflowId: string
  readonly sourceWorkflowRevision?: string | null
  readonly sourcePublicationId?: string | null
  readonly sourcePublicationDigest?: string | null
  readonly configuration: DeploymentSetupConfiguration
}

export type DeploymentSetupCheckpoint =
  | {
    readonly kind: "source_published"
    readonly publicationId: string
    readonly publicationDigest: string
  }
  | {
    readonly kind: "package_exported"
    readonly packageId: string
    readonly packageDigest: string
  }
  | {
    readonly kind: "project_resolved"
    readonly projectId: string
    readonly environmentId: string
  }
  | {
    readonly kind: "release_verified"
    readonly releaseId: string
  }
  | { readonly kind: "credentials_ready" }
  | {
    readonly kind: "runtime_bound"
    readonly operationalDeploymentId: string
  }
  | {
    readonly kind: "activation_requested"
    readonly promotionId: string
    readonly operationalDeploymentId?: string | null
  }
  | {
    readonly kind: "failed"
    readonly failureCode: string
    readonly failureMessage: string
  }
  | { readonly kind: "resumed" }
  | { readonly kind: "abandoned" }

export interface CheckpointDeploymentSetupInput {
  readonly setupId: string
  readonly expectedVersion: number
  readonly operationKey: string
  readonly checkpoint: DeploymentSetupCheckpoint
}

export interface DeploymentSetupResult {
  readonly setup: DeploymentSetup
  readonly replayed?: boolean
}

export interface DeploymentSetupsResult {
  readonly setups: readonly DeploymentSetup[]
}

export async function createDeploymentSetup(
  profile: RelayCloudProfile,
  input: CreateDeploymentSetupInput,
): Promise<DeploymentSetupResult> {
  return postJson(profile, "/deployment-setups", {
    accountId: profile.accountId,
    clientRequestId: input.clientRequestId,
    origin: input.origin,
    sourceSessionId: input.sourceSessionId,
    sourceWorkflowId: input.sourceWorkflowId,
    sourceWorkflowRevision: input.sourceWorkflowRevision ?? null,
    sourcePublicationId: input.sourcePublicationId ?? null,
    sourcePublicationDigest: input.sourcePublicationDigest ?? null,
    configuration: input.configuration,
  })
}

export async function listDeploymentSetups(
  profile: RelayCloudProfile,
): Promise<DeploymentSetupsResult> {
  return getJson(profile, "/deployment-setups", { accountId: profile.accountId })
}

export async function getDeploymentSetup(
  profile: RelayCloudProfile,
  setupId: string,
): Promise<DeploymentSetupResult> {
  return getJson(profile, setupPath(setupId), { accountId: profile.accountId })
}

export async function checkpointDeploymentSetup(
  profile: RelayCloudProfile,
  input: CheckpointDeploymentSetupInput,
): Promise<DeploymentSetupResult> {
  return postJson(profile, `${setupPath(input.setupId)}/checkpoints`, {
    accountId: profile.accountId,
    expectedVersion: input.expectedVersion,
    operationKey: input.operationKey,
    checkpoint: input.checkpoint,
  })
}

function setupPath(setupId: string): string {
  return `/deployment-setups/${encodeURIComponent(setupId)}`
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
