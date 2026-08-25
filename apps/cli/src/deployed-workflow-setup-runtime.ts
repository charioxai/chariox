import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"

import {
  bindWorkflowPublicationDeploymentRequest,
  createWorkflowPublicationRequest,
  exportWorkflowPublicationPackageRequest,
  listWorkflowPublicationsRequest,
  writeWorkflowPublicationExportPackage,
  type WorkflowPublicationDefinition,
  type WorkflowPublicationPackageFile,
} from "@chariox/kernel-client"

import {
  createDeploymentProject,
  createDeploymentRelease,
  getDeploymentAudience,
  getDeploymentEnvironmentCredentials,
  getDeploymentProject,
  getPublicationCallerClaimsVerifier,
  listDeploymentProjects,
  promoteDeploymentRelease,
  setDeploymentAudiencePolicy,
  upsertDeploymentAudienceGrant,
} from "./deployed-workflow-api.js"
import { registerPublicationDeploymentLocalBackend } from "./publication-deployment-api.js"
import { preparePublicationReleasePackage } from "./deployed-workflow-package.js"
import { publicationTransportKind } from "./deployed-workflow-setup-options.js"
import type { DeploymentSetup } from "./deployed-workflow-setup-api.js"
import {
  executeDeploymentSetup,
  type DeploymentSetupExecutionOutcome,
} from "./deployed-workflow-setup-executor.js"
import type { DeploymentProjectKind } from "./deployed-workflow-types.js"
import type { RuntimeSession } from "./cli-types.js"
import type { RelayCloudProfile } from "./preferences.js"

export interface AttachedDeploymentSetupRuntime {
  readonly sessionState: () => RuntimeSession
  readonly sendDeploymentSetupKernelRequest: (
    request: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>
}

export async function runDeploymentSetupRuntime(
  profile: RelayCloudProfile,
  setupId: string,
  runtime: AttachedDeploymentSetupRuntime,
  agentAppAssets?: string,
): Promise<DeploymentSetupExecutionOutcome> {
  let preparedPackage: Awaited<ReturnType<typeof prepareSetupPackage>> | null = null
  const exportPreparedPackage = async (setup: DeploymentSetup) => {
    if (preparedPackage) return preparedPackage
    preparedPackage = await prepareSetupPackage(setup, runtime, agentAppAssets)
    return preparedPackage
  }
  try {
    return await executeDeploymentSetup(profile, setupId, {
      publishSource: (setup) => publishSetupSource(setup, runtime),
      exportPackage: async (setup) => {
        const prepared = await exportPreparedPackage(setup)
        return { packageId: prepared.packageId, packageDigest: prepared.packageDigest }
      },
      resolveProject: async (setup) => {
        const project = await resolveSetupProject(profile, setup)
        await applySetupAccess(profile, setup, project)
        return project
      },
      verifyRelease: async (setup) => {
        const prepared = await exportPreparedPackage(setup)
        const result = await createDeploymentRelease(
          profile,
          requiredText(setup.projectId, "deployment project ID"),
          prepared.root,
        )
        if (result.release.packageId && result.release.packageId !== prepared.packageId) {
          throw new Error("Cloud returned a release for another publication package")
        }
        if (result.release.packageDigest !== prepared.packageDigest) {
          throw new Error("Cloud returned a release with another publication package digest")
        }
        return { releaseId: result.release.id }
      },
      credentialsReady: async (setup) => {
        const result = await getDeploymentEnvironmentCredentials(profile, {
          projectId: requiredText(setup.projectId, "deployment project ID"),
          environmentId: requiredText(setup.environmentId, "deployment environment ID"),
          releaseId: requiredText(setup.releaseId, "deployment release ID"),
        })
        return result.credentials.ready
      },
      bindRuntime: async (setup) => bindSetupRuntime(profile, setup, runtime),
      activateHosted: async (setup) => {
        const result = await promoteSetup(profile, setup, setup.operationKeys.promotion)
        return {
          promotionId: result.promotionId,
          operationalDeploymentId: result.operationalDeploymentId,
        }
      },
    })
  } finally {
    await cleanupPreparedPackage(preparedPackage)
  }
}

async function publishSetupSource(
  setup: DeploymentSetup,
  runtime: AttachedDeploymentSetupRuntime,
): Promise<{ readonly publicationId: string; readonly publicationDigest: string }> {
  if (setup.origin === "publication") {
    return {
      publicationId: requiredText(setup.sourcePublicationId, "deployment setup publication ID"),
      publicationDigest: requiredSha256(setup.sourcePublicationDigest, "immutable publication snapshot digest"),
    }
  }
  const revision = requiredRevision(setup.sourceWorkflowRevision)
  const config = setup.configuration.publication
  const listed = parsePublications(await runtime.sendDeploymentSetupKernelRequest(
    listWorkflowPublicationsRequest(setup.sourceSessionId),
  ))
  const existing = listed.find((publication) => reusablePublication(publication, setup, revision))
  const publication = existing ?? parseCreatedPublication(
    await runtime.sendDeploymentSetupKernelRequest(createWorkflowPublicationRequest(
      setup.sourceSessionId,
      setup.sourceWorkflowId,
      setup.configuration.endpointId,
      {
        expectedWorkflowRevision: revision,
        operationKey: setup.operationKeys.publication,
        alias: config.alias,
        queueRef: config.queueRef ?? null,
        kind: config.kind,
        route: config.route ?? null,
        methods: [...(config.methods ?? [])],
        transport: config.transport ?? null,
        parser: config.parser ?? null,
        inputSchema: config.inputSchema ?? null,
        traceExposure: config.traceExposure ?? null,
        mode: config.mode ?? null,
        syncTimeoutMs: config.syncTimeoutMs ?? null,
        pollMs: config.pollMs ?? null,
      },
    )),
  )
  return {
    publicationId: publication.id,
    publicationDigest: requiredSha256(
      publication.source_snapshot_digest,
      "immutable publication snapshot digest",
    ),
  }
}

async function prepareSetupPackage(
  setup: DeploymentSetup,
  runtime: AttachedDeploymentSetupRuntime,
  agentAppAssets?: string,
) {
  const publicationId = requiredText(setup.sourcePublicationId, "deployment setup publication ID")
  const agentApp = setup.configuration.agentApp?.enabled
    ? setupAgentAppMetadata(setup, publicationId)
    : null
  if (agentApp && !agentAppAssets?.trim()) {
    throw new Error("resume this Agent App deployment with --agent-app-assets <path>")
  }
  const response = await runtime.sendDeploymentSetupKernelRequest(
    exportWorkflowPublicationPackageRequest(setup.sourceSessionId, publicationId, {
      agentApp,
      agentAppAssetsDir: agentAppAssets?.trim() || null,
    }),
  )
  const exported = parsePackageExport(response)
  const root = await mkdtemp(join(tmpdir(), "chariox-deployment-setup-"))
  try {
    await writeWorkflowPublicationExportPackage(root, exported.packageFiles)
    const prepared = await preparePublicationReleasePackage(root)
    if (prepared.packageDigest !== exported.packageDigest) {
      throw new Error("kernel publication package digest does not match its files")
    }
    if (setup.packageId && prepared.packageId !== setup.packageId) {
      throw new Error("publication package identity changed while resuming deployment")
    }
    if (setup.packageDigest && prepared.packageDigest !== setup.packageDigest) {
      throw new Error("publication package digest changed while resuming deployment")
    }
    return { root, packageId: prepared.packageId, packageDigest: prepared.packageDigest }
  } catch (error) {
    await rm(root, { recursive: true, force: true })
    throw error
  }
}

async function cleanupPreparedPackage(prepared: { readonly root: string } | null): Promise<void> {
  if (prepared) await rm(prepared.root, { recursive: true, force: true })
}

async function resolveSetupProject(
  profile: RelayCloudProfile,
  setup: DeploymentSetup,
): Promise<{ readonly projectId: string; readonly environmentId: string }> {
  const deployment = setup.configuration.deployment
  const listed = await listDeploymentProjects(profile)
  const existing = listed.projects.find((project) => project.slug === deployment.slug)
  if (existing && existing.kind !== deployment.kind) {
    throw new Error(`deployment slug ${deployment.slug} belongs to another project kind`)
  }
  const state = existing
    ? (await getDeploymentProject(profile, existing.id)).state
    : (await createDeploymentProject(profile, {
      name: deployment.name,
      slug: deployment.slug,
      kind: deployment.kind as DeploymentProjectKind,
      defaultRuntimeMode: deployment.runtimeMode,
      ...(deployment.region ? { defaultRegion: deployment.region } : {}),
    })).state
  const environment = state.environments.find((candidate) => (
    candidate.slug === state.project.defaultEnvironmentSlug || candidate.tier === "production"
  ))
  if (!environment) throw new Error("deployment project does not have a production environment")
  if (environment.runtimeMode !== deployment.runtimeMode) {
    throw new Error(`deployment ${state.project.slug} uses another runtime mode; choose another slug`)
  }
  return { projectId: state.project.id, environmentId: environment.id }
}

async function applySetupAccess(
  profile: RelayCloudProfile,
  setup: DeploymentSetup,
  project: { readonly projectId: string; readonly environmentId: string },
): Promise<void> {
  const access = setup.configuration.access ?? { kind: "current_account" }
  let audience = (await getDeploymentAudience(profile, project.projectId, project.environmentId)).audience
  const targetMode = access.kind === "public" ? "public" : "restricted"
  const targetDefaultRoles = access.kind === "public" ? ["public"] : []
  if (audience.mode !== targetMode || !sameRoles(audience.defaultRoles, targetDefaultRoles)) {
    audience = (await setDeploymentAudiencePolicy(profile, {
      projectId: project.projectId,
      environmentId: project.environmentId,
      mode: targetMode,
      defaultRoles: targetDefaultRoles,
    })).audience
  }
  if (access.kind !== "email" && access.kind !== "email_domain") return
  const grantExists = audience.grants.some((grant) => (
    grant.kind === access.kind
    && grant.subject === access.subject
    && grant.status === "active"
    && !grant.revokedAt
    && grant.roles.includes("public")
  ))
  if (grantExists) return
  await upsertDeploymentAudienceGrant(profile, {
    projectId: project.projectId,
    environmentId: project.environmentId,
    kind: access.kind,
    subject: access.subject,
    roles: ["public"],
    status: "active",
  })
}

function sameRoles(left: readonly string[], right: readonly string[]): boolean {
  const normalizedLeft = [...left].sort()
  const normalizedRight = [...right].sort()
  return normalizedLeft.length === normalizedRight.length
    && normalizedLeft.every((role, index) => role === normalizedRight[index])
}

async function bindSetupRuntime(
  profile: RelayCloudProfile,
  setup: DeploymentSetup,
  runtime: AttachedDeploymentSetupRuntime,
): Promise<{ readonly operationalDeploymentId: string; readonly state: "running" | "waiting_for_relay" }> {
  const promoted = await promoteSetup(profile, setup, setup.operationKeys.runtime)
  const verifier = await getPublicationCallerClaimsVerifier(profile)
  if (verifier.algorithm !== "Ed25519") {
    throw new Error("Cloud publication caller-claims verifier must use Ed25519")
  }
  const deploymentId = requiredText(promoted.operationalDeploymentId, "operational deployment ID")
  const response = await runtime.sendDeploymentSetupKernelRequest(bindWorkflowPublicationDeploymentRequest(
    setup.sourceSessionId,
    requiredText(setup.sourcePublicationId, "deployment setup publication ID"),
    {
      setupId: setup.id,
      operationKey: setup.operationKeys.runtime,
      deploymentId,
      environmentId: requiredText(setup.environmentId, "deployment environment ID"),
      releaseId: requiredText(setup.releaseId, "deployment release ID"),
      packageDigest: requiredSha256(setup.packageDigest, "deployment package digest"),
      desiredRevision: promoted.desiredRevision,
      callerClaimsPublicKeyPem: requiredCallerClaimsPublicKeyPem(
        verifier.publicKeyPem,
        "publication caller-claims public key",
      ),
    },
  ))
  const payload = variant(response, "WorkflowPublicationDeploymentBound")
  if (
    payload.operation_key !== setup.operationKeys.runtime
    || payload.deployment_id !== deploymentId
    || payload.release_id !== setup.releaseId
    || payload.package_digest !== setup.packageDigest
  ) {
    throw new Error("kernel returned deployment binding facts for another setup")
  }
  if (payload.state !== "running" && payload.state !== "waiting_for_relay") {
    throw new Error("kernel returned an invalid deployment binding state")
  }
  if (payload.state === "running" && publicationUsesHttpIngress(setup.configuration.publication.transport)) {
    try {
      await registerPublicationDeploymentLocalBackend({
        profile,
        deploymentId,
        runtimeSessionId: requiredText(payload.runtime_session_id, "deployment runtime session ID"),
        tunnelUrl: requiredText(payload.tunnel_url, "deployment tunnel URL"),
      })
    } catch (cause) {
      throw new Error(
        "deployment runtime is bound, but ingress backend registration failed; resume this setup to retry registration safely",
        { cause },
      )
    }
  }
  return { operationalDeploymentId: deploymentId, state: payload.state }
}

function publicationUsesHttpIngress(transport: unknown): boolean {
  // Schedule-only and event-based publications run without a public HTTP tunnel.
  return Boolean(
    transport
    && typeof transport === "object"
    && !Array.isArray(transport)
    && (transport as Record<string, unknown>).kind === "human_http",
  )
}

async function promoteSetup(
  profile: RelayCloudProfile,
  setup: DeploymentSetup,
  idempotencyKey: string,
): Promise<{
  readonly promotionId: string
  readonly operationalDeploymentId: string | null
  readonly desiredRevision: number
}> {
  const result = await promoteDeploymentRelease(profile, {
    projectId: requiredText(setup.projectId, "deployment project ID"),
    environmentId: requiredText(setup.environmentId, "deployment environment ID"),
    releaseId: requiredText(setup.releaseId, "deployment release ID"),
    idempotencyKey,
  })
  if (!Number.isSafeInteger(result.environment.desiredRevision) || result.environment.desiredRevision < 0) {
    throw new Error("Cloud returned an invalid deployment revision")
  }
  return {
    promotionId: result.promotion.id,
    operationalDeploymentId: result.environment.operationalDeploymentId ?? null,
    desiredRevision: result.environment.desiredRevision,
  }
}

function setupAgentAppMetadata(setup: DeploymentSetup, publicationId: string): Record<string, unknown> {
  const config = setup.configuration.agentApp
  const routePath = config?.routePath?.trim() || "/app/*"
  const manipulationLevel = config?.manipulationLevel ?? "state_and_overlay"
  const replicaCount = Math.max(1, Math.floor(config?.replicaCount ?? 1))
  return {
    enabled: true,
    assets: { public_dir: "app", index: "index.html" },
    routes: [{
      path: routePath,
      hook_id: `${publicationId}-hook`,
      prompt_source: "path_tail",
      response: "streaming_shell",
      required_role: "public",
      manipulation: {
        level: manipulationLevel,
        scope: "session",
        allowed_paths: ["/generated/**", "/views/**", "/src/**", "/app/**"],
        protected_paths: ["/auth/**", "/payments/**", "/secrets/**"],
        allowed_actions: [],
      },
    }],
    actions: {},
    replicas: { count: replicaCount, per_caller_ordering: true, max_queue_depth: 100 },
    persistent_patch: { enabled: false },
  }
}

function reusablePublication(
  publication: WorkflowPublicationDefinition,
  setup: DeploymentSetup,
  revision: number,
): boolean {
  const config = setup.configuration.publication
  return publication.enabled
    && publication.workflow_id === setup.sourceWorkflowId
    && publication.endpoint_id === setup.configuration.endpointId
    && publication.alias === config.alias
    && publication.route === config.route
    && publication.mode === config.mode
    && publication.source_workflow_revision === revision
    && publicationTransportKind(publication.transport) === publicationTransportKind(config.transport)
    && /^sha256:[a-f0-9]{64}$/.test(publication.source_snapshot_digest ?? "")
}

function parsePublications(response: Record<string, unknown>): WorkflowPublicationDefinition[] {
  const payload = variant(response, "WorkflowPublicationsListed")
  if (!Array.isArray(payload.publications)) throw new Error("kernel did not return workflow triggers")
  return payload.publications.filter(objectRecord) as unknown as WorkflowPublicationDefinition[]
}

function parseCreatedPublication(response: Record<string, unknown>): WorkflowPublicationDefinition {
  const publication = objectRecord(variant(response, "WorkflowPublicationCreated").publication)
  if (typeof publication?.id !== "string") throw new Error("kernel did not return a workflow trigger")
  return publication as unknown as WorkflowPublicationDefinition
}

function parsePackageExport(response: Record<string, unknown>): {
  readonly packageDigest: string
  readonly packageFiles: WorkflowPublicationPackageFile[]
} {
  const payload = variant(response, "WorkflowPublicationPackageExported")
  if (payload.package_version !== 4) throw new Error("kernel publication export must use package version 4")
  const packageDigest = requiredSha256(payload.package_digest, "kernel publication package digest")
  if (!Array.isArray(payload.package_files)) throw new Error("kernel did not return publication package files")
  const packageFiles = payload.package_files.map((candidate) => {
    const file = objectRecord(candidate)
    if (typeof file?.path !== "string" || typeof file.content_base64 !== "string") {
      throw new Error("kernel returned an invalid publication package file")
    }
    return {
      path: file.path,
      content_base64: file.content_base64,
      ...(file.executable === true ? { executable: true } : {}),
    }
  })
  return { packageDigest, packageFiles }
}

function requiredRevision(value: string | null | undefined): number {
  if (typeof value !== "string" || !/^(?:0|[1-9][0-9]*)$/.test(value)) {
    throw new Error("deployment setup workflow revision is invalid")
  }
  const revision = Number(value)
  if (!Number.isSafeInteger(revision)) throw new Error("deployment setup workflow revision is invalid")
  return revision
}

function requiredSha256(value: unknown, label: string): string {
  if (typeof value !== "string" || !/^sha256:[a-f0-9]{64}$/.test(value)) {
    throw new Error(`${label} is invalid`)
  }
  return value
}

function requiredText(value: unknown, label: string): string {
  if (typeof value !== "string" || !value.trim()) throw new Error(`${label} is unavailable`)
  return value.trim()
}

function requiredCallerClaimsPublicKeyPem(value: unknown, label: string): string {
  return `${requiredText(value, label)}\n`
}

function variant(response: Record<string, unknown>, name: string): Record<string, unknown> {
  const payload = objectRecord(response[name])
  if (!payload) throw new Error(`kernel did not return ${name}`)
  return payload
}

function objectRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}
