import assert from "node:assert/strict"
import { rm } from "node:fs/promises"
import test from "node:test"

import {
  adoptLegacyDeploymentProject,
  createDeploymentProject,
  createDeploymentRelease,
  getDeploymentProject,
  listDeploymentProjects,
  promoteDeploymentRelease,
  rollbackDeploymentEnvironment,
} from "./deployed-workflow-api.js"
import { deployedWorkflowPackageFixture } from "./deployed-workflow-package.test-support.js"
import type { RelayCloudProfile } from "./preferences.js"

test("deployed workflow API scopes project and lifecycle requests to the linked account", async () => {
  const originalFetch = globalThis.fetch
  const packageRoot = await deployedWorkflowPackageFixture()
  const calls: Array<{
    readonly method: string
    readonly url: URL
    readonly authorization: string | null
    readonly body: Record<string, unknown> | null
  }> = []
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    calls.push({
      method: init?.method ?? "GET",
      url,
      authorization: new Headers(init?.headers).get("authorization"),
      body,
    })
    if (url.pathname.endsWith("/releases")) {
      return jsonResponse({ release: release(body) })
    }
    if (url.pathname.endsWith("/promotions")) {
      return jsonResponse(promotionResult("promotion-1", body))
    }
    if (url.pathname.endsWith("/rollbacks")) {
      return jsonResponse(promotionResult("rollback-1", body))
    }
    if (url.pathname.endsWith("/legacy-adoptions") || (url.pathname === "/deployment-projects" && init?.method === "POST")) {
      return jsonResponse({ state: projectState() }, 201)
    }
    if (url.pathname === "/deployment-projects") {
      return jsonResponse({ projects: [projectState().project], portfolio: [portfolioItem()] })
    }
    return jsonResponse({ state: projectState() })
  }

  try {
    await listDeploymentProjects(profile)
    await getDeploymentProject(profile, "project/one")
    await createDeploymentProject(profile, {
      name: "Demo",
      slug: "demo",
      kind: "agent_app",
      defaultRuntimeMode: "hosted_container",
      defaultRegion: "fsn1",
    })
    await adoptLegacyDeploymentProject(profile, "legacy-1")
    await createDeploymentRelease(profile, "project-1", packageRoot)
    await promoteDeploymentRelease(profile, {
      projectId: "project-1",
      environmentId: "environment-1",
      releaseId: "release-1",
      idempotencyKey: "promote-key",
      configuration: { feature: true },
      limits: { maxConcurrency: 2 },
    })
    await rollbackDeploymentEnvironment(profile, {
      projectId: "project-1",
      environmentId: "environment-1",
      promotionId: "promotion-1",
      idempotencyKey: "rollback-key",
    })

    assert.equal(calls.length, 7)
    assert.ok(calls.every((call) => call.authorization === "Bearer session-token"))
    assert.equal(calls[0]?.url.searchParams.get("accountId"), "account-1")
    assert.equal(calls[1]?.url.pathname, "/deployment-projects/project%2Fone")
    assert.deepEqual(calls[2]?.body, {
      accountId: "account-1",
      name: "Demo",
      slug: "demo",
      kind: "agent_app",
      defaultRuntimeMode: "hosted_container",
      defaultRegion: "fsn1",
    })
    assert.deepEqual(calls[3]?.body, { accountId: "account-1", deploymentId: "legacy-1" })
    assert.equal(calls[4]?.url.pathname, "/deployment-projects/project-1/releases")
    assert.equal(calls[4]?.body?.accountId, "account-1")
    assert.equal(calls[4]?.body?.packageVersion, 3)
    assert.equal(calls[4]?.body?.contractVersion, 1)
    assert.match(String(calls[4]?.body?.packageId), /^sha256:[a-f0-9]{64}$/)
    assert.match(String(calls[4]?.body?.packageDigest), /^sha256:[a-f0-9]{64}$/)
    assert.equal(typeof (calls[4]?.body?.artifact as Record<string, unknown>)?.archiveBase64, "string")
    assert.deepEqual(calls[5]?.body, {
      accountId: "account-1",
      releaseId: "release-1",
      idempotencyKey: "promote-key",
      configuration: { feature: true },
      limits: { maxConcurrency: 2 },
    })
    assert.deepEqual(calls[6]?.body, {
      accountId: "account-1",
      promotionId: "promotion-1",
      idempotencyKey: "rollback-key",
    })
  } finally {
    globalThis.fetch = originalFetch
    await rm(packageRoot, { recursive: true, force: true })
  }
})

const profile: RelayCloudProfile = {
  apiUrl: "https://cloud.example.test/",
  email: "user@example.test",
  accountId: "account-1",
  userId: "user-1",
  accountSlug: "account",
  realmId: "realm-1",
  relayUrl: "wss://relay.example.test",
  issuerId: "issuer-1",
  cloudSessionToken: "session-token",
}

function projectState() {
  return {
    project: {
      id: "project-1",
      accountId: "account-1",
      name: "Demo",
      slug: "demo",
      kind: "agent_app",
      origin: "native",
      defaultEnvironmentSlug: "production",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
    },
    releases: [],
    environments: [environment()],
    promotions: [],
  }
}

function portfolioItem() {
  return {
    project: projectState().project,
    defaultEnvironment: environment(),
    latestRelease: null,
    latestPromotion: null,
    needsAttention: false,
  }
}

function environment() {
  return {
    id: "environment-1",
    projectId: "project-1",
    name: "Production",
    slug: "production",
    tier: "production",
    runtimeMode: "hosted_container",
    desiredState: "live",
    observedState: "deploying",
    desiredReleaseId: "release-1",
    observedReleaseId: null,
    desiredRevision: 1,
    observedRevision: 0,
    publicUrl: "https://demo.example.test/",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }
}

function release(body: Record<string, unknown> | null) {
  return {
    id: "release-1",
    projectId: "project-1",
    sequence: 1,
    status: "available",
    packageId: body?.packageId,
    packageDigest: body?.packageDigest,
    packageVersion: 3,
    contractVersion: 1,
    verifiedAt: "2026-01-01T00:00:00.000Z",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }
}

function promotionResult(id: string, body: Record<string, unknown> | null) {
  return {
    promotion: {
      id,
      projectId: "project-1",
      environmentId: "environment-1",
      fromReleaseId: null,
      toReleaseId: body?.releaseId ?? "release-0",
      desiredRevision: 1,
      status: "pending",
      requestedAt: "2026-01-01T00:00:00.000Z",
    },
    environment: environment(),
  }
}

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  })
}
