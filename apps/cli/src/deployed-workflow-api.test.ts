import assert from "node:assert/strict"
import { rm } from "node:fs/promises"
import test from "node:test"

import {
  acceptDeploymentClaim,
  adoptLegacyDeploymentProject,
  bindDeploymentEnvironmentCredential,
  changeDeploymentEnvironmentLifecycle,
  createDeploymentClaim,
  createDeploymentCredentialProfile,
  createDeploymentProject,
  createDeploymentRelease,
  getDeploymentAccess,
  getDeploymentEnvironmentCredentials,
  getDeploymentProject,
  listDeploymentProjects,
  listDeploymentCredentialProfiles,
  promoteDeploymentRelease,
  reviewDeploymentClaim,
  requestDeploymentCredentialOperation,
  revokeDeploymentClaim,
  revokeDeploymentProjectMember,
  revokeDeploymentEnvironmentCredentialBinding,
  rollbackDeploymentEnvironment,
  upsertDeploymentProjectMember,
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
    if (url.pathname.endsWith("/start") || url.pathname.endsWith("/stop") || url.pathname.endsWith("/restart")) {
      return jsonResponse({ environment: environment() }, 202)
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
    for (const action of ["stop", "start", "restart"] as const) {
      await changeDeploymentEnvironmentLifecycle(profile, {
        projectId: "project-1",
        environmentId: "environment-1",
        action,
        idempotencyKey: `${action}-key`,
      })
    }

    assert.equal(calls.length, 10)
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
    assert.deepEqual(calls.slice(7).map((call) => [call.method, call.url.pathname, call.body]), [
      ["POST", "/deployment-projects/project-1/environments/environment-1/stop", {
        accountId: "account-1",
        idempotencyKey: "stop-key",
      }],
      ["POST", "/deployment-projects/project-1/environments/environment-1/start", {
        accountId: "account-1",
        idempotencyKey: "start-key",
      }],
      ["POST", "/deployment-projects/project-1/environments/environment-1/restart", {
        accountId: "account-1",
        idempotencyKey: "restart-key",
      }],
    ])
  } finally {
    globalThis.fetch = originalFetch
    await rm(packageRoot, { recursive: true, force: true })
  }
})

test("deployed workflow API scopes claim and member handoff requests", async () => {
  const originalFetch = globalThis.fetch
  const calls: Array<{ readonly method: string; readonly url: URL; readonly body: Record<string, unknown> | null }> = []
  globalThis.fetch = async (input, init) => {
    calls.push({
      method: init?.method ?? "GET",
      url: new URL(String(input)),
      body: typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null,
    })
    return jsonResponse({
      claim: claimSummary(),
      claimToken: "arroba_claim_secret",
      state: projectState(),
      access: accessState(),
    })
  }
  try {
    await createDeploymentClaim(profile, {
      projectId: "project/one",
      releaseId: "release-1",
      ownershipMode: "customer_owned",
      builderRole: "maintainer",
      targetAccountId: "customer-account",
      targetEmail: "owner@customer.test",
      expiresInSeconds: 600,
    })
    await reviewDeploymentClaim(profile, "arroba_claim_secret")
    await acceptDeploymentClaim(profile, {
      claimToken: "arroba_claim_secret",
      projectName: "Customer App",
      projectSlug: "customer-app",
      runtimeMode: "local_runtime",
    })
    await revokeDeploymentClaim(profile, "project/one", "claim/one")
    await getDeploymentAccess(profile, "project/one")
    await upsertDeploymentProjectMember(profile, {
      projectId: "project/one",
      granteeAccountId: "support-account",
      userEmail: "support@example.test",
      role: "operator",
    })
    await revokeDeploymentProjectMember(profile, "project/one", "member/one")

    assert.deepEqual(calls.map((call) => [call.method, call.url.pathname]), [
      ["POST", "/deployment-projects/project%2Fone/claims"],
      ["POST", "/deployment-claims/review"],
      ["POST", "/deployment-claims/accept"],
      ["POST", "/deployment-projects/project%2Fone/claims/claim%2Fone/revoke"],
      ["GET", "/deployment-projects/project%2Fone/access"],
      ["POST", "/deployment-projects/project%2Fone/members"],
      ["POST", "/deployment-projects/project%2Fone/members/member%2Fone/revoke"],
    ])
    assert.deepEqual(calls[0]?.body, {
      accountId: "account-1",
      releaseId: "release-1",
      ownershipMode: "customer_owned",
      builderRole: "maintainer",
      targetAccountId: "customer-account",
      targetEmail: "owner@customer.test",
      expiresInSeconds: 600,
    })
    assert.deepEqual(calls[2]?.body, {
      accountId: "account-1",
      claimToken: "arroba_claim_secret",
      projectName: "Customer App",
      projectSlug: "customer-app",
      runtimeMode: "local_runtime",
    })
    assert.equal(calls[4]?.url.searchParams.get("accountId"), "account-1")
    assert.deepEqual(calls[5]?.body, {
      accountId: "account-1",
      granteeAccountId: "support-account",
      userEmail: "support@example.test",
      role: "operator",
    })
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow API scopes the destination credential lifecycle", async () => {
  const originalFetch = globalThis.fetch
  const calls: Array<{ readonly method: string; readonly url: URL; readonly body: Record<string, unknown> | null }> = []
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    calls.push({ method: init?.method ?? "GET", url, body })
    if (url.pathname.endsWith("/credentials") || url.pathname.includes("/credential-bindings")) {
      return jsonResponse({ credentials: credentialState() })
    }
    if (url.pathname === "/deployment-credentials" && !init?.method) {
      return jsonResponse({ profiles: [credentialProfile()] })
    }
    const type = url.pathname === "/deployment-credentials"
      ? "connect"
      : url.pathname.split("/").at(-1) ?? "test"
    return jsonResponse({ profile: credentialProfile(), job: credentialJob(type) }, 202)
  }
  try {
    await listDeploymentCredentialProfiles(profile)
    await createDeploymentCredentialProfile(profile, {
      kind: "provider",
      provider: "codex",
      label: "Production Codex",
    })
    for (const operation of ["test", "rotate", "revoke", "purge"] as const) {
      await requestDeploymentCredentialOperation(profile, "profile/one", operation)
    }
    await getDeploymentEnvironmentCredentials(profile, {
      projectId: "project/one",
      environmentId: "environment/one",
      releaseId: "release/one",
    })
    await bindDeploymentEnvironmentCredential(profile, {
      projectId: "project/one",
      environmentId: "environment/one",
      releaseId: "release/one",
      slotId: "provider:codex",
      profileId: "profile/one",
    })
    await revokeDeploymentEnvironmentCredentialBinding(profile, {
      projectId: "project/one",
      environmentId: "environment/one",
      slotId: "provider:codex",
    })

    assert.deepEqual(calls.map((call) => [call.method, call.url.pathname]), [
      ["GET", "/deployment-credentials"],
      ["POST", "/deployment-credentials"],
      ["POST", "/deployment-credentials/profile%2Fone/test"],
      ["POST", "/deployment-credentials/profile%2Fone/rotate"],
      ["POST", "/deployment-credentials/profile%2Fone/revoke"],
      ["POST", "/deployment-credentials/profile%2Fone/purge"],
      ["GET", "/deployment-projects/project%2Fone/environments/environment%2Fone/credentials"],
      ["POST", "/deployment-projects/project%2Fone/environments/environment%2Fone/credential-bindings"],
      ["POST", "/deployment-projects/project%2Fone/environments/environment%2Fone/credential-bindings/revoke"],
    ])
    assert.equal(calls[0]?.url.searchParams.get("accountId"), "account-1")
    assert.deepEqual(calls[1]?.body, {
      accountId: "account-1",
      kind: "provider",
      provider: "codex",
      label: "Production Codex",
    })
    assert.ok(calls.slice(2, 6).every((call) => call.body?.accountId === "account-1"))
    assert.equal(calls[6]?.url.searchParams.get("releaseId"), "release/one")
    assert.deepEqual(calls[7]?.body, {
      accountId: "account-1",
      releaseId: "release/one",
      slotId: "provider:codex",
      profileId: "profile/one",
    })
    assert.deepEqual(calls[8]?.body, { accountId: "account-1", slotId: "provider:codex" })
  } finally {
    globalThis.fetch = originalFetch
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
    control: {
      role: "maintainer",
      source: "project_member",
      canRead: true,
      canRelease: true,
      canOperate: true,
      canManage: false,
    },
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

function claimSummary() {
  return {
    id: "claim-1",
    sourceAccountId: "account-1",
    sourceProjectId: "project-1",
    sourceReleaseId: "release-1",
    sourceProjectName: "Demo",
    sourceProjectSlug: "demo",
    sourceReleaseSequence: 1,
    sourcePackageDigest: "sha256:package",
    createdByUserId: "user-1",
    ownershipMode: "customer_owned",
    builderRole: "maintainer",
    tokenPrefix: "arroba_claim_",
    status: "pending",
    expiresAt: "2026-01-02T00:00:00.000Z",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }
}

function accessState() {
  return {
    projectId: "project-1",
    projectAccountId: "account-1",
    ownershipMode: "customer_owned",
    builderAccountId: "builder-account",
    claims: [claimSummary()],
    members: [],
  }
}

function credentialProfile() {
  return {
    id: "profile-1",
    accountId: "account-1",
    kind: "provider",
    provider: "codex",
    label: "Production Codex",
    accountLabel: "customer@example.test",
    version: 2,
    status: "ready",
    runnerConnected: true,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }
}

function credentialJob(type: string) {
  return {
    id: `job-${type}`,
    accountId: "account-1",
    profileId: "profile-1",
    type,
    status: "pending",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }
}

function credentialState() {
  return {
    projectId: "project-1",
    environmentId: "environment-1",
    releaseId: "release-1",
    ready: true,
    slots: [{
      slot: {
        slotId: "provider:codex",
        kind: "provider",
        label: "Codex provider",
        provider: "codex",
        required: true,
        scope: "environment",
        uses: ["agent:primary"],
        testMethod: "native_auth",
      },
      readiness: "ready",
      binding: {
        id: "binding-1",
        profileId: "profile-1",
        version: 2,
        status: "active",
        profile: credentialProfile(),
      },
    }],
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
