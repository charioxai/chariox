import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { readFile, rm, stat } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type { RuntimeSession, WorkflowPublicationDefinition } from "@chariox/kernel-client"

import { executeDeploymentSetupCommand } from "./deployed-workflow-setup-command.js"
import { preparePublicationReleasePackage } from "./deployed-workflow-package.js"
import { deployedWorkflowPackageFixture } from "./deployed-workflow-package.test-support.js"
import type {
  DeploymentSetup,
  DeploymentSetupCheckpoint,
  DeploymentSetupConfiguration,
} from "./deployed-workflow-setup-api.js"
import type { RelayCloudProfile } from "./preferences.js"

const sourceDigest = `sha256:${"a".repeat(64)}`

test("TUI deployment setup publishes a draft and binds a local runtime", async () => {
  const fixture = await setupFixture({ mode: "local_runtime", bindStates: ["running"] })
  try {
    const output = await executeDeploymentSetupCommand(profile, [
      "draft", "workflow-1", "endpoint-1",
      "--slug", "demo-local",
      "--transport", "human-http",
      "--mode", "local-runtime",
      "--client-request-id", "draft-local-request",
    ], fixture.runtime)

    assert.equal(output.footer, "deployment demo-local ready")
    assert.match(output.notice, /status completed/)
    assert.match(output.notice, /^request_id=draft-local-request$/m)
    assert.match(output.notice, /deployment deployment-1/)
    assert.deepEqual(fixture.kernelVariants, [
      "ListWorkflowPublications",
      "CreateWorkflowPublication",
      "ExportWorkflowPublicationPackage",
      "BindWorkflowPublicationDeployment",
    ])
    assert.equal(fixture.cloud.setup?.sourceWorkflowRevision, "7")
    assert.equal(fixture.cloud.setup?.configuration.publication.alias, "demo-local-r7")
    assert.equal(fixture.cloud.promotionKeys[0], fixture.cloud.setup?.operationKeys.runtime)
    assert.equal(fixture.cloud.checkpoints.at(-1)?.kind, "runtime_bound")
  } finally {
    await fixture.cleanup()
  }
})

test("TUI deployment setup deploys an immutable publication to a hosted container", async () => {
  const fixture = await setupFixture({ mode: "hosted_container" })
  try {
    const output = await executeDeploymentSetupCommand(profile, [
      "publication", "published-demo",
      "--slug", "demo-hosted",
      "--mode", "hosted-container",
      "--client-request-id", "published-hosted-request",
    ], fixture.runtime)

    assert.equal(output.footer, "deployment demo-hosted ready")
    assert.match(output.notice, /^request_id=published-hosted-request$/m)
    assert.equal(fixture.kernelVariants.includes("CreateWorkflowPublication"), false)
    assert.deepEqual(fixture.kernelVariants, ["ExportWorkflowPublicationPackage"])
    assert.equal(fixture.cloud.setup?.origin, "publication")
    assert.equal(fixture.cloud.setup?.configuration.deployment.region, "eu-central")
    assert.equal(fixture.cloud.promotionKeys[0], fixture.cloud.setup?.operationKeys.promotion)
    assert.equal(fixture.cloud.checkpoints.at(-1)?.kind, "activation_requested")
  } finally {
    await fixture.cleanup()
  }
})

test("TUI deployment setup persists and applies verified-domain access", async () => {
  const fixture = await setupFixture({ mode: "hosted_container" })
  try {
    const output = await executeDeploymentSetupCommand(profile, [
      "publication", "published-demo",
      "--slug", "domain-hosted",
      "--mode", "hosted-container",
      "--access", "verified-domain",
      "--access-subject", "@Example.COM",
    ], fixture.runtime)

    assert.equal(output.footer, "deployment domain-hosted ready")
    assert.match(output.notice, /access verified-domain:example\.com/)
    assert.deepEqual(fixture.cloud.setup?.configuration.access, {
      kind: "email_domain",
      subject: "example.com",
    })
    assert.deepEqual(fixture.cloud.audienceMutations, [{
      kind: "email_domain",
      subject: "example.com",
      roles: ["public"],
      status: "active",
    }])
  } finally {
    await fixture.cleanup()
  }
})

test("TUI deployment setup pauses for credentials and resumes without re-exporting", async () => {
  const fixture = await setupFixture({ mode: "hosted_container", credentialsReady: false })
  try {
    const first = await executeDeploymentSetupCommand(profile, [
      "publication", "publication-1",
      "--slug", "credential-pause",
      "--mode", "hosted-container",
    ], fixture.runtime)
    assert.equal(first.footer, "deployment setup awaits credentials")
    assert.equal(fixture.cloud.setup?.stage, "credentials")
    assert.equal(fixture.kernelVariants.filter((name) => name === "ExportWorkflowPublicationPackage").length, 1)

    fixture.cloud.credentialsReady = true
    const resumed = await executeDeploymentSetupCommand(profile, [
      "resume", fixture.cloud.setup!.id,
    ], fixture.runtime)
    assert.equal(resumed.footer, "deployment credential-pause ready")
    assert.equal(fixture.kernelVariants.filter((name) => name === "ExportWorkflowPublicationPackage").length, 1)
  } finally {
    await fixture.cleanup()
  }
})

test("TUI local setup pauses for a missing relay and resumes idempotently", async () => {
  const fixture = await setupFixture({
    mode: "local_runtime",
    bindStates: ["waiting_for_relay", "running"],
  })
  try {
    const first = await executeDeploymentSetupCommand(profile, [
      "publication", "publication-1",
      "--slug", "relay-resume",
      "--mode", "local-runtime",
    ], fixture.runtime)
    assert.equal(first.footer, "deployment setup waits for relay runtime")
    assert.equal(fixture.cloud.setup?.stage, "runtime")
    assert.equal(fixture.cloud.checkpoints.some((checkpoint) => checkpoint.kind === "runtime_bound"), false)

    const resumed = await executeDeploymentSetupCommand(profile, [
      "resume", fixture.cloud.setup!.id,
    ], fixture.runtime)
    assert.equal(resumed.footer, "deployment relay-resume ready")
    assert.equal(fixture.cloud.promotionKeys.length, 2)
    assert.equal(fixture.cloud.promotionKeys[0], fixture.cloud.promotionKeys[1])
    assert.equal(fixture.cloud.promotionKeys[0], fixture.cloud.setup?.operationKeys.runtime)
    assert.equal(fixture.cloud.checkpoints.at(-1)?.kind, "runtime_bound")
  } finally {
    await fixture.cleanup()
  }
})

test("TUI deployment setup rejects package paths that escape its temporary root", async () => {
  const escapeName = `chariox-setup-escape-${randomUUID()}`
  const fixture = await setupFixture({ mode: "hosted_container", unsafePackagePath: `../${escapeName}` })
  const escapedPath = join(tmpdir(), escapeName)
  try {
    await assert.rejects(
      executeDeploymentSetupCommand(profile, [
        "publication", "publication-1",
        "--slug", "unsafe-package",
        "--mode", "hosted-container",
      ], fixture.runtime),
      /unsafe path/,
    )
    await assert.rejects(stat(escapedPath), { code: "ENOENT" })
    assert.equal(fixture.cloud.setup?.stage, "package")
  } finally {
    await rm(escapedPath, { force: true })
    await fixture.cleanup()
  }
})

async function setupFixture(options: {
  readonly mode: "local_runtime" | "hosted_container"
  readonly credentialsReady?: boolean
  readonly bindStates?: Array<"running" | "waiting_for_relay">
  readonly unsafePackagePath?: string
}) {
  const packageRoot = await deployedWorkflowPackageFixture()
  const prepared = await preparePublicationReleasePackage(packageRoot)
  const packageFiles = await Promise.all([
    "publication.json",
    "deployment-contract.json",
    "public/index.html",
    "run.sh",
  ].map(async (path) => ({
    path,
    content_base64: (await readFile(join(packageRoot, path))).toString("base64"),
    ...(path === "run.sh" ? { executable: true } : {}),
  })))
  if (options.unsafePackagePath) packageFiles[0] = { ...packageFiles[0]!, path: options.unsafePackagePath }

  const cloud = new FakeDeploymentCloud(options.mode, options.credentialsReady ?? true)
  const kernelVariants: string[] = []
  const bindStates = [...(options.bindStates ?? ["running"])]
  const publication = publicationFixture()
  const sendKernelRequest = async (request: Record<string, unknown>): Promise<Record<string, unknown>> => {
    const name = Object.keys(request)[0]!
    kernelVariants.push(name)
    switch (name) {
      case "ListWorkflowPublications": return { WorkflowPublicationsListed: { publications: [] } }
      case "CreateWorkflowPublication": return { WorkflowPublicationCreated: { publication } }
      case "ExportWorkflowPublicationPackage": return {
        WorkflowPublicationPackageExported: {
          publication,
          package_version: 3,
          package_digest: prepared.packageDigest,
          package_archive_base64: prepared.artifact.archiveBase64,
          package_files: packageFiles,
        },
      }
      case "BindWorkflowPublicationDeployment": {
        const input = request.BindWorkflowPublicationDeployment as Record<string, unknown>
        return {
          WorkflowPublicationDeploymentBound: {
            ...input,
            state: bindStates.shift() ?? "running",
          },
        }
      }
      default: throw new Error(`unexpected kernel request ${name}`)
    }
  }
  const session = sessionFixture(publication)
  const runtime = {
    isAttached: () => true,
    sessionState: () => session,
    sendDeploymentSetupKernelRequest: sendKernelRequest,
  }
  const originalFetch = globalThis.fetch
  globalThis.fetch = cloud.fetch
  return {
    cloud,
    kernelVariants,
    runtime,
    cleanup: async () => {
      globalThis.fetch = originalFetch
      await rm(packageRoot, { recursive: true, force: true })
    },
  }
}

class FakeDeploymentCloud {
  setup: DeploymentSetup | null = null
  credentialsReady: boolean
  readonly checkpoints: DeploymentSetupCheckpoint[] = []
  readonly promotionKeys: string[] = []
  readonly audienceMutations: Record<string, unknown>[] = []
  private audience = {
    mode: "restricted" as "public" | "restricted",
    defaultRoles: [] as string[],
    routes: [] as unknown[],
    grants: [{
      id: "owner-grant",
      policyId: "audience-environment-1",
      kind: "account" as const,
      subject: "account-1",
      roles: ["public"],
      status: "active" as const,
      revokedAt: null,
      createdAt: timestamp,
      updatedAt: timestamp,
    }] as Array<Record<string, unknown>>,
    apiKeys: [] as unknown[],
    jwtIssuers: [] as unknown[],
    webhookKeys: [] as unknown[],
    id: "audience-environment-1",
    accountId: "account-1",
    projectId: "project-1",
    environmentId: "environment-1",
    createdAt: timestamp,
    updatedAt: timestamp,
  }

  constructor(
    private readonly mode: "local_runtime" | "hosted_container",
    credentialsReady: boolean,
  ) {
    this.credentialsReady = credentialsReady
  }

  readonly fetch = async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input))
    const method = init?.method ?? "GET"
    const body = typeof init?.body === "string"
      ? JSON.parse(init.body) as Record<string, unknown>
      : null
    if (url.pathname === "/deployment-setups" && method === "POST") {
      const configuration = body?.configuration as DeploymentSetupConfiguration
      this.setup = setupRecord({
        origin: body?.origin as "draft" | "publication",
        clientRequestId: String(body?.clientRequestId),
        sourceSessionId: String(body?.sourceSessionId),
        sourceWorkflowId: String(body?.sourceWorkflowId),
        sourceWorkflowRevision: body?.sourceWorkflowRevision == null ? null : String(body.sourceWorkflowRevision),
        sourcePublicationId: body?.sourcePublicationId == null ? null : String(body.sourcePublicationId),
        sourcePublicationDigest: body?.sourcePublicationDigest == null ? null : String(body.sourcePublicationDigest),
        configuration,
      })
      return jsonResponse({ setup: this.setup }, 201)
    }
    if (url.pathname === "/deployment-setups" && method === "GET") {
      return jsonResponse({ setups: this.setup ? [this.setup] : [] })
    }
    if (/^\/deployment-setups\/[^/]+$/.test(url.pathname) && method === "GET") {
      return jsonResponse({ setup: this.requiredSetup() })
    }
    if (url.pathname.endsWith("/checkpoints") && method === "POST") {
      const checkpoint = body?.checkpoint as DeploymentSetupCheckpoint
      this.checkpoints.push(checkpoint)
      this.setup = checkpointSetup(this.requiredSetup(), checkpoint)
      return jsonResponse({ setup: this.setup })
    }
    if (url.pathname === "/deployment-projects" && method === "GET") {
      return jsonResponse({ projects: [], portfolio: [] })
    }
    if (url.pathname === "/deployment-projects" && method === "POST") {
      return jsonResponse({ state: projectState(this.requiredSetup()) }, 201)
    }
    if (url.pathname.endsWith("/audience") && method === "GET") {
      return jsonResponse({ audience: this.audience })
    }
    if (url.pathname.endsWith("/audience/policy") && method === "POST") {
      this.audience = {
        ...this.audience,
        mode: body?.mode as "public" | "restricted",
        defaultRoles: body?.defaultRoles as string[],
      }
      return jsonResponse({ audience: this.audience })
    }
    if (url.pathname.endsWith("/audience/grants") && method === "POST") {
      const mutation = {
        kind: body?.kind,
        subject: body?.subject,
        roles: body?.roles,
        status: body?.status,
      }
      this.audienceMutations.push(mutation)
      this.audience = {
        ...this.audience,
        grants: [...this.audience.grants, {
          id: `grant-${this.audience.grants.length + 1}`,
          policyId: this.audience.id,
          ...mutation,
          revokedAt: null,
          createdAt: timestamp,
          updatedAt: timestamp,
        }],
      }
      return jsonResponse({ audience: this.audience }, 201)
    }
    if (url.pathname.endsWith("/releases") && method === "POST") {
      return jsonResponse({ release: {
        id: "release-1",
        projectId: "project-1",
        sequence: 1,
        status: "verified",
        packageId: body?.packageId,
        packageDigest: body?.packageDigest,
        packageVersion: 3,
        createdAt: timestamp,
        updatedAt: timestamp,
      } }, 201, { "x-request-id": "release-request" })
    }
    if (url.pathname.endsWith("/credentials") && method === "GET") {
      return jsonResponse({ credentials: {
        projectId: "project-1",
        environmentId: "environment-1",
        releaseId: "release-1",
        ready: this.credentialsReady,
        slots: [],
      } })
    }
    if (url.pathname.endsWith("/promotions") && method === "POST") {
      this.promotionKeys.push(String(body?.idempotencyKey))
      return jsonResponse({
        promotion: {
          id: "promotion-1",
          projectId: "project-1",
          environmentId: "environment-1",
          toReleaseId: "release-1",
          desiredRevision: 1,
          status: "requested",
          requestedAt: timestamp,
        },
        environment: environment(this.mode),
      }, 202, { "x-request-id": "promotion-request" })
    }
    return jsonResponse({ error: { message: `unexpected ${method} ${url.pathname}` } }, 500)
  }

  private requiredSetup(): DeploymentSetup {
    if (!this.setup) throw new Error("setup has not been created")
    return this.setup
  }
}

function setupRecord(input: {
  readonly origin: "draft" | "publication"
  readonly clientRequestId: string
  readonly sourceSessionId: string
  readonly sourceWorkflowId: string
  readonly sourceWorkflowRevision: string | null
  readonly sourcePublicationId: string | null
  readonly sourcePublicationDigest: string | null
  readonly configuration: DeploymentSetupConfiguration
}): DeploymentSetup {
  const id = "setup-1"
  return {
    id,
    accountId: profile.accountId,
    createdByUserId: "user-1",
    clientRequestId: input.clientRequestId,
    origin: input.origin,
    status: "active",
    stage: input.origin === "draft" ? "source" : "package",
    version: 0,
    sourceSessionId: input.sourceSessionId,
    sourceWorkflowId: input.sourceWorkflowId,
    sourceWorkflowRevision: input.sourceWorkflowRevision,
    sourcePublicationId: input.sourcePublicationId,
    sourcePublicationDigest: input.sourcePublicationDigest,
    configuration: input.configuration,
    createdAt: timestamp,
    updatedAt: timestamp,
    operationKeys: {
      publication: `${id}:publication`,
      package: `${id}:package`,
      project: `${id}:project`,
      release: `${id}:release`,
      credentials: `${id}:credentials`,
      promotion: `${id}:promotion`,
      runtime: `${id}:runtime`,
    },
  }
}

function checkpointSetup(setup: DeploymentSetup, checkpoint: DeploymentSetupCheckpoint): DeploymentSetup {
  const base = { ...setup, version: setup.version + 1, updatedAt: timestamp }
  switch (checkpoint.kind) {
    case "source_published": return {
      ...base,
      stage: "package",
      sourcePublicationId: checkpoint.publicationId,
      sourcePublicationDigest: checkpoint.publicationDigest,
    }
    case "package_exported": return {
      ...base,
      stage: "project",
      packageId: checkpoint.packageId,
      packageDigest: checkpoint.packageDigest,
    }
    case "project_resolved": return {
      ...base,
      stage: "release",
      projectId: checkpoint.projectId,
      environmentId: checkpoint.environmentId,
    }
    case "release_verified": return { ...base, stage: "credentials", releaseId: checkpoint.releaseId }
    case "credentials_ready": return {
      ...base,
      stage: setup.configuration.deployment.runtimeMode === "local_runtime" ? "runtime" : "activation",
    }
    case "runtime_bound": return {
      ...base,
      status: "completed",
      stage: "complete",
      operationalDeploymentId: checkpoint.operationalDeploymentId,
      completedAt: timestamp,
    }
    case "activation_requested": return {
      ...base,
      status: "completed",
      stage: "complete",
      promotionId: checkpoint.promotionId,
      ...(checkpoint.operationalDeploymentId !== undefined
        ? { operationalDeploymentId: checkpoint.operationalDeploymentId }
        : {}),
      completedAt: timestamp,
    }
    case "failed": return {
      ...base,
      status: "blocked",
      failureCode: checkpoint.failureCode,
      failureMessage: checkpoint.failureMessage,
    }
    case "resumed": return { ...base, status: "active" }
    case "abandoned": return { ...base, status: "abandoned", abandonedAt: timestamp }
  }
}

function projectState(setup: DeploymentSetup) {
  return {
    project: {
      id: "project-1",
      accountId: profile.accountId,
      name: setup.configuration.deployment.name,
      slug: setup.configuration.deployment.slug,
      kind: setup.configuration.deployment.kind,
      origin: "native",
      defaultEnvironmentSlug: "production",
      createdAt: timestamp,
      updatedAt: timestamp,
    },
    releases: [],
    environments: [environment(setup.configuration.deployment.runtimeMode)],
    promotions: [],
  }
}

function environment(mode: "local_runtime" | "hosted_container") {
  return {
    id: "environment-1",
    projectId: "project-1",
    name: "Production",
    slug: "production",
    tier: "production",
    runtimeMode: mode,
    region: mode === "hosted_container" ? "eu-central" : null,
    desiredState: "running",
    observedState: "pending",
    desiredReleaseId: "release-1",
    observedReleaseId: null,
    desiredRevision: 1,
    observedRevision: 0,
    operationalDeploymentId: "deployment-1",
    publicUrl: "https://chariox-cloud-staging.osc-fr1.scalingo.io/deployments/demo",
    createdAt: timestamp,
    updatedAt: timestamp,
  }
}

function publicationFixture(): WorkflowPublicationDefinition {
  return {
    id: "publication-1",
    session_id: "session-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    alias: "published-demo",
    enabled: true,
    kind: "ingress",
    route: "/prompt/*",
    methods: ["GET"],
    transport: { kind: "human_http" },
    parser: { kind: "path_template", template: "/prompt/:prompt" },
    mode: "async",
    source_workflow_revision: 7,
    source_snapshot_digest: sourceDigest,
    created_by_user_id: "user-1",
    created_at_ms: 1,
    updated_at_ms: 1,
  }
}

function sessionFixture(publication: WorkflowPublicationDefinition): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 1,
    status: "active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 4,
    agents: [],
    config_state: { version: 0, values: {}, updated_by_attachment_id: null },
    workflows: [{
      id: "workflow-1",
      alias: "demo-workflow",
      revision: 7,
      nodes: [],
      edges: [],
      endpoints: [{ id: "endpoint-1", alias: "prompt", entry_node_id: "node-1" }],
    }],
    workflow_publications: [publication],
  }
}

function jsonResponse(value: unknown, status = 200, headers?: HeadersInit): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json", ...headers },
  })
}

const profile: RelayCloudProfile = {
  apiUrl: "https://chariox-cloud-staging.osc-fr1.scalingo.io",
  email: "user@example.test",
  relayUrl: "wss://relay.scalingo.test",
  accountId: "account-1",
  userId: "user-1",
  accountSlug: "account",
  realmId: "realm-1",
  issuerId: "issuer-1",
  cloudSessionToken: "cloud-session",
}

const timestamp = "2026-07-18T00:00:00.000Z"
