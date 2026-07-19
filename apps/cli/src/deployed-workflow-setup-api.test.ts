import assert from "node:assert/strict"
import test from "node:test"

import {
  checkpointDeploymentSetup,
  createDeploymentSetup,
  getDeploymentSetup,
  listDeploymentSetups,
  type DeploymentSetup,
  type DeploymentSetupCheckpoint,
  type DeploymentSetupConfiguration,
} from "./deployed-workflow-setup-api.js"
import type { RelayCloudProfile } from "./preferences.js"

test("deployment setup API scopes, authenticates, encodes, and preserves replay responses", async () => {
  const originalFetch = globalThis.fetch
  const calls: Array<{
    readonly method: string
    readonly url: URL
    readonly headers: Headers
    readonly body: Record<string, unknown> | null
  }> = []

  globalThis.fetch = async (input, init) => {
    const method = init?.method ?? "GET"
    const url = new URL(String(input))
    calls.push({
      method,
      url,
      headers: new Headers(init?.headers),
      body: typeof init?.body === "string"
        ? JSON.parse(init.body) as Record<string, unknown>
        : null,
    })

    if (method === "POST" && url.pathname.endsWith("/checkpoints")) {
      return jsonResponse({ setup: { ...setup, version: 2 }, replayed: true })
    }
    if (method === "POST") return jsonResponse({ setup, replayed: false }, 201)
    if (url.pathname === "/deployment-setups") return jsonResponse({ setups: [setup] })
    return jsonResponse({ setup })
  }

  try {
    const created = await createDeploymentSetup(profile, {
      clientRequestId: "request-1",
      origin: "draft",
      sourceSessionId: "session-1",
      sourceWorkflowId: "workflow-1",
      sourceWorkflowRevision: "7",
      configuration,
    })
    const listed = await listDeploymentSetups(profile)
    const loaded = await getDeploymentSetup(profile, "setup / 1")
    const checkpointed = []
    for (const [index, checkpoint] of checkpointVariants.entries()) {
      checkpointed.push(await checkpointDeploymentSetup(profile, {
        setupId: "setup / 1",
        expectedVersion: index + 1,
        operationKey: `operation-${index + 1}`,
        checkpoint,
      }))
    }

    assert.equal(created.replayed, false)
    assert.equal(listed.setups[0]?.createdAt, setup.createdAt)
    assert.equal(loaded.setup.id, setup.id)
    assert.ok(checkpointed.every((result) => result.replayed === true))
    assert.ok(calls.every((call) => call.headers.get("authorization") === "Bearer session-token"))
    assert.ok(calls.every((call) => call.headers.get("accept") === "application/json"))
    assert.ok(calls.every((call) => call.headers.get("content-type") === "application/json"))

    assert.deepEqual(calls.slice(0, 3).map((call) => [call.method, call.url.pathname]), [
      ["POST", "/deployment-setups"],
      ["GET", "/deployment-setups"],
      ["GET", "/deployment-setups/setup%20%2F%201"],
    ])
    assert.equal(calls[1]?.url.searchParams.get("accountId"), "account / current")
    assert.equal(calls[2]?.url.searchParams.get("accountId"), "account / current")
    assert.deepEqual(calls[0]?.body, {
      accountId: "account / current",
      clientRequestId: "request-1",
      origin: "draft",
      sourceSessionId: "session-1",
      sourceWorkflowId: "workflow-1",
      sourceWorkflowRevision: "7",
      sourcePublicationId: null,
      sourcePublicationDigest: null,
      configuration,
    })

    const checkpointCalls = calls.slice(3)
    assert.ok(checkpointCalls.every((call) => (
      call.method === "POST"
      && call.url.pathname === "/deployment-setups/setup%20%2F%201/checkpoints"
    )))
    assert.deepEqual(checkpointCalls.map((call) => call.body), checkpointVariants.map((checkpoint, index) => ({
      accountId: "account / current",
      expectedVersion: index + 1,
      operationKey: `operation-${index + 1}`,
      checkpoint,
    })))
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployment setup API surfaces Cloud error messages and status fallbacks", async () => {
  const originalFetch = globalThis.fetch

  try {
    globalThis.fetch = async () => jsonResponse({
      error: {
        code: "deployment_setup_version_conflict",
        message: "Deployment setup changed; reload it before continuing.",
      },
    }, 409)

    await assert.rejects(
      checkpointDeploymentSetup(profile, {
        setupId: "setup-1",
        expectedVersion: 1,
        operationKey: "operation-1",
        checkpoint: { kind: "credentials_ready" },
      }),
      /Deployment setup changed; reload it before continuing\./,
    )

    globalThis.fetch = async () => new Response("temporarily unavailable", { status: 503 })
    await assert.rejects(
      listDeploymentSetups(profile),
      /deployed workflow request failed with 503/,
    )
  } finally {
    globalThis.fetch = originalFetch
  }
})

const checkpointVariants = [
  {
    kind: "source_published",
    publicationId: "publication-1",
    publicationDigest: digest("a"),
  },
  {
    kind: "package_exported",
    packageId: digest("b"),
    packageDigest: digest("c"),
  },
  {
    kind: "project_resolved",
    projectId: "project-1",
    environmentId: "environment-1",
  },
  { kind: "release_verified", releaseId: "release-1" },
  { kind: "credentials_ready" },
  { kind: "runtime_bound", operationalDeploymentId: "deployment-1" },
  {
    kind: "activation_requested",
    promotionId: "promotion-1",
    operationalDeploymentId: "deployment-1",
  },
  {
    kind: "failed",
    failureCode: "credential_unavailable",
    failureMessage: "Credential enrollment is incomplete.",
  },
  { kind: "resumed" },
  { kind: "abandoned" },
] satisfies readonly DeploymentSetupCheckpoint[]

const configuration: DeploymentSetupConfiguration = {
  endpointId: "endpoint-1",
  publication: {
    alias: "support",
    kind: "http",
    route: "/support",
    methods: ["POST"],
    transport: { kind: "json" },
  },
  deployment: {
    name: "Support app",
    slug: "support-app",
    kind: "agent_app",
    runtimeMode: "hosted_container",
    region: "eu-central",
  },
  agentApp: {
    enabled: true,
    routePath: "/",
    manipulationLevel: "parameters",
    replicaCount: 1,
  },
}

const setup: DeploymentSetup = {
  id: "setup / 1",
  accountId: "account / current",
  createdByUserId: "user-1",
  clientRequestId: "request-1",
  origin: "draft",
  status: "active",
  stage: "source",
  version: 1,
  sourceSessionId: "session-1",
  sourceWorkflowId: "workflow-1",
  sourceWorkflowRevision: "7",
  sourcePublicationId: null,
  sourcePublicationDigest: null,
  configuration,
  packageId: null,
  packageDigest: null,
  projectId: null,
  releaseId: null,
  environmentId: null,
  promotionId: null,
  operationalDeploymentId: null,
  failureCode: null,
  failureMessage: null,
  completedAt: null,
  abandonedAt: null,
  createdAt: "2026-07-18T20:00:00.000Z",
  updatedAt: "2026-07-18T20:01:00.000Z",
  operationKeys: {
    publication: "deployment-setup:setup-1:publication",
    package: "deployment-setup:setup-1:package",
    project: "deployment-setup:setup-1:project",
    release: "deployment-setup:setup-1:release",
    credentials: "deployment-setup:setup-1:credentials",
    promotion: "deployment-setup:setup-1:promotion",
    runtime: "deployment-setup:setup-1:runtime",
  },
}

const profile: RelayCloudProfile = {
  apiUrl: "https://cloud.example.test/",
  email: "user@example.test",
  accountId: "account / current",
  userId: "user-1",
  accountSlug: "account",
  realmId: "realm-1",
  relayUrl: "wss://relay.example.test",
  issuerId: "issuer-1",
  cloudSessionToken: "session-token",
}

function digest(character: string): string {
  return `sha256:${character.repeat(64)}`
}

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  })
}
