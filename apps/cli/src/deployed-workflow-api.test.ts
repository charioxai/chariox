import assert from "node:assert/strict"
import { rm } from "node:fs/promises"
import test from "node:test"

import {
  acceptDeploymentClaim,
  bindDeploymentEnvironmentCredential,
  changeDeploymentEnvironmentLifecycle,
  createDeploymentClaim,
  createDeploymentCredentialProfile,
  createDeploymentProject,
  createDeploymentRelease,
  deleteDeploymentEnvironmentTelemetry,
  exportDeploymentEnvironmentTelemetry,
  getDeploymentAccess,
  getDeploymentCredentialEnrollment,
  getDeploymentEnvironmentCredentials,
  getDeploymentEnvironmentUsage,
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
  updateDeploymentEnvironmentLimits,
  waitForDeploymentCredentialEnrollment,
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
    if (url.pathname === "/deployment-projects" && init?.method === "POST") {
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
    const createdRelease = await createDeploymentRelease(profile, "project-1", packageRoot)
    const promoted = await promoteDeploymentRelease(profile, {
      projectId: "project-1",
      environmentId: "environment-1",
      releaseId: "release-1",
      idempotencyKey: "promote-key",
      configuration: { feature: true },
      limits: { concurrency: 2 },
    })
    const rolledBack = await rollbackDeploymentEnvironment(profile, {
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

    assert.equal(createdRelease.requestId, "request-1")
    assert.equal(promoted.requestId, "request-1")
    assert.equal(rolledBack.requestId, "request-1")
    assert.equal(calls.length, 9)
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
    assert.equal(calls[3]?.url.pathname, "/deployment-projects/project-1/releases")
    assert.equal(calls[3]?.body?.accountId, "account-1")
    assert.equal(calls[3]?.body?.packageVersion, 4)
    assert.equal(calls[3]?.body?.contractVersion, 1)
    assert.match(String(calls[3]?.body?.packageId), /^sha256:[a-f0-9]{64}$/)
    assert.match(String(calls[3]?.body?.packageDigest), /^sha256:[a-f0-9]{64}$/)
    assert.equal(typeof (calls[3]?.body?.artifact as Record<string, unknown>)?.archiveBase64, "string")
    assert.deepEqual(calls[4]?.body, {
      accountId: "account-1",
      releaseId: "release-1",
      idempotencyKey: "promote-key",
      configuration: { feature: true },
      limits: { concurrency: 2 },
    })
    assert.deepEqual(calls[5]?.body, {
      accountId: "account-1",
      promotionId: "promotion-1",
      idempotencyKey: "rollback-key",
    })
    assert.deepEqual(calls.slice(6).map((call) => [call.method, call.url.pathname, call.body]), [
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
      claimToken: "chariox_claim_secret",
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
    await reviewDeploymentClaim(profile, "chariox_claim_secret")
    await acceptDeploymentClaim(profile, {
      claimToken: "chariox_claim_secret",
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
    assert.deepEqual(calls[1]?.body, {
      accountId: "account-1",
      claimToken: "chariox_claim_secret",
    })
    assert.deepEqual(calls[2]?.body, {
      accountId: "account-1",
      claimToken: "chariox_claim_secret",
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
    if (url.pathname.endsWith("/enrollment")) {
      return jsonResponse({ enrollment: credentialEnrollment() })
    }
    if (url.pathname === "/deployment-credentials" && !init?.method) {
      return jsonResponse({
        profiles: [{ ...credentialProfile(), enrollment: credentialEnrollment() }],
        setupAccess: "available",
      })
    }
    const type = url.pathname === "/deployment-credentials"
      ? "connect"
      : url.pathname.split("/").at(-1) ?? "test"
    const enrollmentActive = type === "connect" || type === "rotate" || type === "setup"
    return jsonResponse({
      profile: {
        ...credentialProfile(),
        ...(enrollmentActive ? {
          status: "connecting",
          enrollment: { ...credentialEnrollment(), verificationUrl: null, userCode: null },
        } : {}),
      },
      job: credentialJob(type),
    }, 202)
  }
  try {
    const listed = await listDeploymentCredentialProfiles(profile)
    const created = await createDeploymentCredentialProfile(profile, {
      kind: "provider",
      provider: "codex",
      label: "Production Codex",
    })
    let rotated: Awaited<ReturnType<typeof requestDeploymentCredentialOperation>> | undefined
    for (const operation of ["test", "rotate", "retry", "revoke", "purge"] as const) {
      const result = await requestDeploymentCredentialOperation(profile, "profile/one", operation)
      if (operation === "rotate") rotated = result
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

    assert.equal(created.setupDetailsStatus, "available")
    assert.equal(created.profile.enrollment?.userCode, "ABCD-1234")
    assert.equal(rotated?.profile.enrollment?.verificationUrl, "https://auth.openai.com/codex/device?user_code=ABCD-1234")
    assert.equal(listed.profiles[0]?.enrollment?.instructions, null)
    assert.equal(listed.profiles[0]?.enrollment?.verificationUrl, null)
    assert.equal(listed.profiles[0]?.enrollment?.userCode, null)

    assert.deepEqual(calls.map((call) => [call.method, call.url.pathname]), [
      ["GET", "/deployment-credentials"],
      ["POST", "/deployment-credentials"],
      ["GET", "/deployment-credentials/profile-1/enrollment"],
      ["POST", "/deployment-credentials/profile%2Fone/test"],
      ["POST", "/deployment-credentials/profile%2Fone/rotate"],
      ["GET", "/deployment-credentials/profile-1/enrollment"],
      ["POST", "/deployment-credentials/profile%2Fone/setup"],
      ["GET", "/deployment-credentials/profile-1/enrollment"],
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
    assert.ok([calls[3], calls[4], calls[6], calls[8], calls[9]].every((call) => call?.body?.accountId === "account-1"))
    assert.equal(calls[10]?.url.searchParams.get("releaseId"), "release/one")
    assert.deepEqual(calls[11]?.body, {
      accountId: "account-1",
      releaseId: "release/one",
      slotId: "provider:codex",
      profileId: "profile/one",
    })
    assert.deepEqual(calls[12]?.body, { accountId: "account-1", slotId: "provider:codex" })
    assert.equal("enrollmentMode" in (calls[1]?.body ?? {}), false)
    assert.equal("enrollmentMode" in (calls[4]?.body ?? {}), false)
    assert.equal("enrollmentMode" in (calls[6]?.body ?? {}), false)
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow API rejects successful non-JSON credential responses", async () => {
  const originalFetch = globalThis.fetch
  globalThis.fetch = async () => new Response("<html>Access login</html>", {
    status: 200,
    headers: { "content-type": "text/html" },
  })

  try {
    await assert.rejects(
      listDeploymentCredentialProfiles(profile),
      /deployed workflow request returned non-JSON HTTP 200/,
    )
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow enrollment details propagate member authorization denial", async () => {
  const originalFetch = globalThis.fetch
  globalThis.fetch = async () => jsonResponse({
    error: { message: "Credential enrollment setup requires account owner or admin access" },
  }, 403)
  try {
    await assert.rejects(
      () => getDeploymentCredentialEnrollment(profile, "profile/member"),
      /requires account owner or admin access/,
    )
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("successful credential setup mutations report unavailable privileged details", async () => {
  const originalFetch = globalThis.fetch
  let request = 0
  globalThis.fetch = async () => {
    request += 1
    if (request === 1) {
      return jsonResponse({
        profile: {
          ...credentialProfile(),
          status: "connecting",
          enrollment: { ...credentialEnrollment(), verificationUrl: null, userCode: null },
        },
        job: credentialJob("connect"),
      }, 201)
    }
    return jsonResponse({ error: { message: "credential setup details unavailable" } }, 503)
  }
  try {
    const result = await createDeploymentCredentialProfile(profile, {
      kind: "provider",
      provider: "codex",
      label: "Production Codex",
    })
    assert.equal(result.profile.id, "profile-1")
    assert.equal(result.setupDetailsStatus, "unavailable")
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow enrollment polling observes actionable and terminal transitions", async () => {
  const originalFetch = globalThis.fetch
  let responseIndex = 0
  globalThis.fetch = async () => {
    responseIndex += 1
    return jsonResponse({
      enrollment: responseIndex === 1
        ? {
            ...credentialEnrollment(),
            status: "pending",
            verificationUrl: null,
            userCode: null,
          }
        : credentialEnrollment(),
    })
  }
  try {
    const actionable = await waitForDeploymentCredentialEnrollment(profile, "profile-1", {
      intervalMs: 0,
      maxAttempts: 3,
    })
    assert.equal(responseIndex, 2)
    assert.equal(actionable.enrollment?.status, "claimed")
    assert.equal(actionable.enrollment?.userCode, "ABCD-1234")

    responseIndex = 0
    globalThis.fetch = async () => {
      responseIndex += 1
      return jsonResponse({
        enrollment: responseIndex === 1
          ? {
              ...credentialEnrollment(),
              status: "pending",
              verificationUrl: null,
              userCode: null,
            }
          : {
              ...credentialEnrollment(),
              status: "expired",
              verificationUrl: null,
              userCode: null,
            },
      })
    }
    const terminal = await waitForDeploymentCredentialEnrollment(profile, "profile-1", {
      intervalMs: 0,
      maxAttempts: 3,
    })
    assert.equal(responseIndex, 2)
    assert.equal(terminal.enrollment?.status, "expired")
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow API scopes runtime usage and limit updates", async () => {
  const originalFetch = globalThis.fetch
  const calls: Array<{ readonly method: string; readonly url: URL; readonly body: Record<string, unknown> | null }> = []
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    calls.push({ method: init?.method ?? "GET", url, body })
    if (init?.method === "POST") {
      if (url.pathname.endsWith("/telemetry/export")) {
        return jsonResponse({
          exportId: "export-1",
          filename: "telemetry.json",
          mediaType: "application/json",
          generatedAt: new Date(0).toISOString(),
          byteSize: 3,
          sha256: `sha256:${"0".repeat(64)}`,
          contentBase64: "e30K",
          counts: { invocationMetadata: 1, deploymentLogs: 2, auditEvents: 3 },
        })
      }
      if (url.pathname.endsWith("/telemetry/delete")) {
        return jsonResponse({
          deletedAt: new Date(0).toISOString(),
          deletedInvocationCount: 1,
          deletedLogCount: 2,
          protectedActiveInvocationCount: 1,
        })
      }
      return jsonResponse({
        environment: { ...environment(), limits: body?.limits, desiredRevision: 2 },
        changed: true,
        restartRequested: true,
      })
    }
    return jsonResponse({ usage: runtimeUsage() })
  }
  try {
    await getDeploymentEnvironmentUsage(profile, "project/one", "environment/one")
    const updated = await updateDeploymentEnvironmentLimits(profile, {
      projectId: "project/one",
      environmentId: "environment/one",
      limits: { concurrency: 4, queue: 8 },
      idempotencyKey: "limits-1",
    })
    assert.equal(updated.restartRequested, true)
    assert.equal((await exportDeploymentEnvironmentTelemetry(
      profile,
      "project/one",
      "environment/one",
    )).exportId, "export-1")
    assert.equal((await deleteDeploymentEnvironmentTelemetry(profile, {
      projectId: "project/one",
      environmentId: "environment/one",
      idempotencyKey: "telemetry-delete-1",
    })).protectedActiveInvocationCount, 1)

    const base = "/deployment-projects/project%2Fone/environments/environment%2Fone"
    assert.deepEqual(calls.map((call) => [call.method, call.url.pathname]), [
      ["GET", `${base}/usage`],
      ["POST", `${base}/limits`],
      ["POST", `${base}/telemetry/export`],
      ["POST", `${base}/telemetry/delete`],
    ])
    assert.equal(calls[0]?.url.searchParams.get("accountId"), "account-1")
    assert.deepEqual(calls[1]?.body, {
      accountId: "account-1",
      idempotencyKey: "limits-1",
      limits: { concurrency: 4, queue: 8 },
    })
    assert.deepEqual(calls[2]?.body, { accountId: "account-1" })
    assert.deepEqual(calls[3]?.body, {
      accountId: "account-1",
      idempotencyKey: "telemetry-delete-1",
    })
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

function runtimeUsage() {
  return {
    accountId: "account-1",
    projectId: "project/one",
    environmentId: "environment/one",
    deploymentId: "deployment-1",
    generatedAt: "2026-01-01T00:00:01.000Z",
    dayStartedAt: "2026-01-01T00:00:00.000Z",
    limits: { concurrency: 2, queue: 8 },
    activeInvocations: 1,
    invocationsLastMinute: 2,
    invocationsToday: 3,
    usageUnitsToday: 3,
    succeededToday: 2,
    failedToday: 1,
    timedOutToday: 0,
    interruptedToday: 0,
    averageDurationMs: 20,
    maximumDurationMs: 30,
    averageQueuedMs: 4,
    requestBytesToday: 100,
    responseBytesToday: 200,
    recentInvocations: [],
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
    tokenPrefix: "chariox_claim_",
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

function credentialEnrollment() {
  return {
    id: "enrollment-1",
    profileId: "profile-1",
    targetVersion: 2,
    mode: "provider_native",
    status: "claimed",
    instructions: "Open the provider verification page.",
    verificationUrl: "https://auth.openai.com/codex/device?user_code=ABCD-1234",
    userCode: "ABCD-1234",
    expiresAt: "2026-07-15T12:30:00.000Z",
    createdAt: "2026-07-15T12:00:00.000Z",
    updatedAt: "2026-07-15T12:01:00.000Z",
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
    packageVersion: 4,
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
    headers: { "content-type": "application/json", "x-request-id": "request-1" },
  })
}
