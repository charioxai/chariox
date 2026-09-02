import { execFile } from "node:child_process"
import { readFile, mkdtemp, rm, stat } from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import { tmpdir } from "node:os"
import { pathToFileURL } from "node:url"
import {
  resolveWorkflowPublicationDeploymentContract,
  workflowPublicationDeploymentContractPath,
  type WorkflowPublicationDeploymentContract,
} from "@chariox/kernel-client/workflow-publication-deployment-contract"
import type { RelayCloudProfile } from "./preferences.js"

export type PublicationDeploymentMode = "local_runtime" | "hosted_container"

export interface PublicationDeploymentSummary {
  readonly id: string
  readonly mode: PublicationDeploymentMode
  readonly slug: string
  readonly publicBaseUrl: string
  readonly status: string
  readonly publicationId: string
  readonly publicationAlias?: string | null
  readonly workflowId?: string | null
  readonly endpointId?: string | null
  readonly hookId?: string | null
  readonly transport: string
  readonly route?: string | null
  readonly agentApp?: unknown
  readonly packageUri?: string | null
  readonly packageVersion?: number | null
  readonly credentialProfile?: string | null
  readonly lastError?: string | null
  readonly lastErrorCode?: string | null
  readonly health?: string | null
  readonly healthCheckedAt?: string | null
  readonly queueDepth?: number | null
  readonly activeReplicaCount?: number | null
  readonly readyReplicaCount?: number | null
}

export interface PublicationPackageMetadata {
  readonly packageRoot: string
  readonly packageUri: string
  readonly packageVersion: number
  readonly publicationId: string
  readonly publicationAlias?: string | null
  readonly workflowId?: string | null
  readonly endpointId?: string | null
  readonly hookId?: string | null
  readonly transport: string
  readonly route?: string | null
  readonly agentApp?: unknown
  readonly deploymentContract?: WorkflowPublicationDeploymentContract
}

export async function createPublicationDeploymentFromPackage(input: {
  readonly profile: RelayCloudProfile
  readonly packagePath: string
  readonly mode: PublicationDeploymentMode
  readonly slug?: string
  readonly credentialProfile?: string
  readonly start?: boolean
}): Promise<PublicationDeploymentSummary> {
  const metadata = await readPublicationPackageMetadata(input.packagePath)
  assertManagedCloudPackagePolicy(metadata.agentApp)
  const created = await postJson<{ readonly deployment: PublicationDeploymentSummary }>(input.profile, "/publication-deployments", {
    accountId: input.profile.accountId,
    mode: input.mode,
    slug: input.slug,
    publicationId: metadata.publicationId,
    publicationAlias: metadata.publicationAlias,
    workflowId: metadata.workflowId,
    endpointId: metadata.endpointId,
    hookId: metadata.hookId,
    transport: metadata.transport,
    route: metadata.route,
    agentApp: metadata.agentApp,
    credentialProfile: input.credentialProfile,
  })
  const digest = await publicationPackageDigest(metadata.packageRoot)
  const packageArchiveBase64 = await publicationPackageArchiveBase64(metadata.packageRoot)
  const uploaded = await uploadPublicationDeploymentPackage({
    profile: input.profile,
    deploymentId: created.deployment.id,
    metadata,
    digest,
    packageArchiveBase64,
  })
  if (input.start || input.mode === "hosted_container") {
    await postJson(input.profile, `/publication-deployments/${encodeURIComponent(uploaded.deployment.id)}/start`, {
      accountId: input.profile.accountId,
    })
  }
  return uploaded.deployment
}

export async function reuploadPublicationDeploymentPackage(input: {
  readonly profile: RelayCloudProfile
  readonly deploymentId: string
  readonly packagePath: string
}): Promise<PublicationDeploymentSummary> {
  const metadata = await readPublicationPackageMetadata(input.packagePath)
  assertManagedCloudPackagePolicy(metadata.agentApp)
  const uploaded = await uploadPublicationDeploymentPackage({
    profile: input.profile,
    deploymentId: input.deploymentId,
    metadata,
    digest: await publicationPackageDigest(metadata.packageRoot),
    packageArchiveBase64: await publicationPackageArchiveBase64(metadata.packageRoot),
  })
  return uploaded.deployment
}

async function uploadPublicationDeploymentPackage(input: {
  readonly profile: RelayCloudProfile
  readonly deploymentId: string
  readonly metadata: PublicationPackageMetadata
  readonly digest: string
  readonly packageArchiveBase64: string
}): Promise<{ readonly deployment: PublicationDeploymentSummary }> {
  return postJson<{ readonly deployment: PublicationDeploymentSummary }>(
    input.profile,
    `/publication-deployments/${encodeURIComponent(input.deploymentId)}/package`,
    {
      accountId: input.profile.accountId,
      packageDigest: input.digest,
      packageVersion: input.metadata.packageVersion,
      packageUri: input.metadata.packageUri,
      packageArchiveBase64: input.packageArchiveBase64,
    },
  )
}

export async function listPublicationDeployments(profile: RelayCloudProfile): Promise<readonly PublicationDeploymentSummary[]> {
  const url = new URL(`${normalizeApiUrl(profile.apiUrl)}/publication-deployments`)
  url.searchParams.set("accountId", profile.accountId)
  const response = await fetch(url, { headers: cloudHeaders(profile) })
  const body = await readJson<{ readonly deployments?: readonly PublicationDeploymentSummary[] }>(response)
  return body.deployments ?? []
}

export async function getPublicationDeployment(profile: RelayCloudProfile, deploymentId: string): Promise<PublicationDeploymentSummary> {
  const url = new URL(`${normalizeApiUrl(profile.apiUrl)}/publication-deployments/${encodeURIComponent(deploymentId)}`)
  url.searchParams.set("accountId", profile.accountId)
  const response = await fetch(url, { headers: cloudHeaders(profile) })
  return (await readJson<{ readonly deployment: PublicationDeploymentSummary }>(response)).deployment
}

export async function changePublicationDeployment(profile: RelayCloudProfile, deploymentId: string, action: "stop" | "restart"): Promise<void> {
  await postJson(profile, `/publication-deployments/${encodeURIComponent(deploymentId)}/${action}`, {
    accountId: profile.accountId,
  })
}

export async function registerPublicationDeploymentLocalBackend(input: {
  readonly profile: RelayCloudProfile
  readonly deploymentId: string
  readonly runtimeSessionId: string
  readonly tunnelUrl: string
}): Promise<void> {
  await postJson(
    input.profile,
    `/publication-deployments/${encodeURIComponent(input.deploymentId)}/local-backend`,
    {
      accountId: input.profile.accountId,
      status: "ready",
      runtimeSessionId: input.runtimeSessionId,
      backendTarget: {
        kind: "local_runtime",
        url: input.tunnelUrl,
        updated_at_ms: Date.now(),
      },
    },
  )
}

export async function listPublicationDeploymentLogs(profile: RelayCloudProfile, deploymentId: string): Promise<readonly { readonly level: string; readonly message: string; readonly occurredAt: string }[]> {
  const url = new URL(`${normalizeApiUrl(profile.apiUrl)}/publication-deployments/${encodeURIComponent(deploymentId)}/logs`)
  url.searchParams.set("accountId", profile.accountId)
  const response = await fetch(url, { headers: cloudHeaders(profile) })
  const body = await readJson<{ readonly logs?: readonly { readonly level: string; readonly message: string; readonly occurredAt: string }[] }>(response)
  return body.logs ?? []
}

export async function readPublicationPackageMetadata(packagePath: string): Promise<PublicationPackageMetadata> {
  const resolved = resolve(packagePath)
  const pathStat = await stat(resolved)
  const packageRoot = pathStat.isDirectory() ? resolved : dirname(resolved)
  const publicationPath = pathStat.isDirectory() ? resolve(packageRoot, "publication.json") : resolved
  const publicationPackage = JSON.parse(await readFile(publicationPath, "utf8")) as {
    package_version?: number
    publication_id?: string
    alias?: string | null
    workflow_id?: string
    hooks?: Array<{
      id?: string
      endpoint_id?: string
      transport?: string
      route?: string
    }>
    agent_app?: unknown
    deployment_contract?: { path?: string; schema_version?: number }
  }
  const deploymentContractPath = workflowPublicationDeploymentContractPath(publicationPackage)
  const deploymentContractValue = deploymentContractPath
    ? JSON.parse(await readFile(resolve(packageRoot, deploymentContractPath), "utf8")) as unknown
    : undefined
  const deploymentContract = resolveWorkflowPublicationDeploymentContract(
    publicationPackage,
    deploymentContractValue,
  )
  const hook = publicationPackage.hooks?.[0]
  if (!publicationPackage.publication_id || !publicationPackage.workflow_id || !hook?.endpoint_id) {
    throw new Error("publication package is missing publication_id, workflow_id, or hook endpoint_id")
  }
  return {
    packageRoot,
    packageUri: pathToFileURL(packageRoot).toString(),
    packageVersion: publicationPackage.package_version ?? 1,
    publicationId: publicationPackage.publication_id,
    workflowId: publicationPackage.workflow_id,
    endpointId: hook.endpoint_id,
    transport: hook.transport ?? "human_http",
    ...(hook.route !== undefined ? { route: hook.route } : {}),
    ...(publicationPackage.agent_app !== undefined ? { agentApp: publicationPackage.agent_app } : {}),
    ...(deploymentContract.kind === "native" ? { deploymentContract: deploymentContract.contract } : {}),
    ...(publicationPackage.alias !== undefined ? { publicationAlias: publicationPackage.alias } : {}),
    ...(hook.id !== undefined ? { hookId: hook.id } : {}),
  }
}

function assertManagedCloudPackagePolicy(agentApp: unknown): void {
  if (managedAgentAppRequestsPersistentPatch(agentApp)) {
    throw new Error("Persistent patches are not available for managed Cloud deployments.")
  }
}

function managedAgentAppRequestsPersistentPatch(agentApp: unknown): boolean {
  const config = objectRecord(agentApp)
  if (!config) return false
  const persistentPatch = config.persistent_patch ?? config.persistentPatch
  if (persistentPatch === true || objectRecord(persistentPatch)?.enabled === true) return true
  if (!Array.isArray(config.routes)) return false
  return config.routes.some((candidate) => {
    const manipulation = objectRecord(objectRecord(candidate)?.manipulation)
    return manipulation?.level === "persistent_patch"
      || manipulation?.level === "persistentPatch"
      || manipulation?.scope === "persistent"
      || manipulation?.scope === "deployment"
  })
}

function objectRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

async function publicationPackageDigest(packageRoot: string): Promise<string> {
  const { createHash } = await import("node:crypto")
  const { readdir } = await import("node:fs/promises")
  const hash = createHash("sha256")
  const files = await readdir(packageRoot, { recursive: true, withFileTypes: true })
  for (const file of files.filter((entry) => entry.isFile()).sort((a, b) => a.name.localeCompare(b.name))) {
    const path = resolve(file.parentPath, file.name)
    hash.update(path.slice(packageRoot.length))
    hash.update(await readFile(path))
  }
  return `sha256:${hash.digest("hex")}`
}

async function publicationPackageArchiveBase64(packageRoot: string): Promise<string> {
  const tempRoot = await mkdtemp(join(tmpdir(), "chariox-publication-upload-"))
  const archivePath = join(tempRoot, "publication-package.tgz")
  try {
    await execFilePromise("tar", ["-czf", archivePath, "-C", packageRoot, "."])
    return (await readFile(archivePath)).toString("base64")
  } finally {
    await rm(tempRoot, { recursive: true, force: true })
  }
}

function execFilePromise(file: string, args: readonly string[]): Promise<void> {
  return new Promise((resolvePromise, reject) => {
    execFile(file, [...args], (error, stdout, stderr) => {
      if (error) {
        reject(new Error(`${file} ${args.join(" ")} failed: ${stderr || stdout || error.message}`))
      } else {
        resolvePromise()
      }
    })
  })
}

async function postJson<TResponse = unknown>(
  profile: RelayCloudProfile,
  pathname: string,
  body: Record<string, unknown>,
): Promise<TResponse> {
  const response = await fetch(`${normalizeApiUrl(profile.apiUrl)}${pathname}`, {
    method: "POST",
    redirect: "manual",
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
      : `publication deployment request failed with ${response.status}`
    throw new Error(message)
  }
  if (body === null) {
    throw new Error(`publication deployment request returned non-JSON HTTP ${response.status}`)
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
