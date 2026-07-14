import assert from "node:assert/strict"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import {
  executeDeployedWorkflowCommand,
  formatDeploymentPortfolioItem,
  handleDeployedWorkflowCloudCommand,
} from "./deployed-workflow-command.js"
import type { DeploymentPortfolioItem } from "./deployed-workflow-types.js"
import type { RelayCloudProfile } from "./preferences.js"

test("deployed workflow command renders portfolio convergence and attention", () => {
  assert.equal(formatDeploymentPortfolioItem(portfolioItem()), [
    "project-1",
    "Demo app",
    "agent_app",
    "ownership=internal_team",
    "production",
    "degraded",
    "release=#2:available",
    "revision=1/2",
    "https://demo.example.test/",
    "attention=required",
  ].join("\t"))
})

test("deployed workflow TUI command drives claim handoff and member access", async () => {
  const originalFetch = globalThis.fetch
  const calls: Array<{ readonly pathname: string; readonly body: Record<string, unknown> | null }> = []
  const notices: string[] = []
  const footers: string[] = []
  globalThis.fetch = async (input, init) => {
    const pathname = new URL(String(input)).pathname
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    calls.push({ pathname, body })
    if (pathname === "/deployment-claims/accept") {
      return jsonResponse({ claim: { ...claimSummary(), status: "accepted", claimedProjectId: "customer-project" }, state: projectState() }, 201)
    }
    if (pathname.endsWith("/access") || pathname.includes("/members")) {
      return jsonResponse({ access: accessState() })
    }
    return jsonResponse({ claim: claimSummary(), claimToken: "arroba_claim_one_time_secret" }, 201)
  }
  try {
    const handled = await handleDeployedWorkflowCloudCommand({
      appendNotice: (message) => notices.push(message),
      flashFooter: (message) => footers.push(message),
    }, profile, "deployments", "claim", [
      "create",
      "project-1",
      "release-2",
      "--ownership",
      "customer-owned",
      "--builder-role",
      "viewer",
      "--target-account",
      "customer-account",
      "--target-email",
      "owner@customer.test",
      "--expires-seconds",
      "600",
    ])
    assert.equal(handled, true)
    assert.match(notices[0] ?? "", /claim_token arroba_claim_one_time_secret/)
    assert.equal(footers[0], "deployment claim created; token shown once")

    await executeDeployedWorkflowCommand(profile, ["claim", "review", "arroba_claim_one_time_secret"])
    const accepted = await executeDeployedWorkflowCommand(profile, [
      "claim",
      "accept",
      "arroba_claim_one_time_secret",
      "--name",
      "Customer app",
      "--slug",
      "customer-app",
      "--mode",
      "local-runtime",
    ])
    const access = await executeDeployedWorkflowCommand(profile, ["access", "project-1"])
    await executeDeployedWorkflowCommand(profile, [
      "member",
      "add",
      "project-1",
      "support-account",
      "support@example.test",
      "operator",
    ])
    await executeDeployedWorkflowCommand(profile, ["member", "revoke", "project-1", "member-1"])

    assert.equal(accepted.footer, "claimed deployment demo")
    assert.doesNotMatch(access.notice, /arroba_claim_one_time_secret/)
    assert.match(access.notice, /member member-1 active/)
    assert.deepEqual(calls[0]?.body, {
      accountId: "account-1",
      releaseId: "release-2",
      ownershipMode: "customer_owned",
      builderRole: "viewer",
      targetAccountId: "customer-account",
      targetEmail: "owner@customer.test",
      expiresInSeconds: 600,
    })
    assert.deepEqual(calls[2]?.body, {
      accountId: "account-1",
      claimToken: "arroba_claim_one_time_secret",
      projectName: "Customer app",
      projectSlug: "customer-app",
      runtimeMode: "local_runtime",
    })
    assert.deepEqual(calls[4]?.body, {
      accountId: "account-1",
      granteeAccountId: "support-account",
      userEmail: "support@example.test",
      role: "operator",
    })
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow TUI command lists projects through the shared Cloud path", async () => {
  const originalFetch = globalThis.fetch
  const notices: string[] = []
  const footers: string[] = []
  globalThis.fetch = async () => jsonResponse({
    projects: [portfolioItem().project],
    portfolio: [portfolioItem()],
  })
  try {
    const handled = await handleDeployedWorkflowCloudCommand({
      appendNotice: (message) => notices.push(message),
      flashFooter: (message) => footers.push(message),
    }, profile, "deployments", "list", [])

    assert.equal(handled, true)
    assert.match(notices[0] ?? "", /project-1\tDemo app\tagent_app/)
    assert.equal(footers[0], "1 deployed workflow")
    assert.equal(await handleDeployedWorkflowCloudCommand({
      appendNotice: () => undefined,
      flashFooter: () => undefined,
    }, profile, "invite", "list", []), false)
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow command parses create and promotion configuration", async () => {
  const originalFetch = globalThis.fetch
  const root = await mkdtemp(join(tmpdir(), "arroba-deployment-command-"))
  const bodies: Record<string, unknown>[] = []
  await writeFile(join(root, "configuration.json"), JSON.stringify({ feature: true }))
  await writeFile(join(root, "limits.json"), JSON.stringify({ maxConcurrency: 3 }))
  globalThis.fetch = async (input, init) => {
    const body = JSON.parse(String(init?.body)) as Record<string, unknown>
    bodies.push(body)
    const pathname = new URL(String(input)).pathname
    if (pathname.endsWith("/promotions")) return jsonResponse(promotionResult())
    return jsonResponse({ state: projectState() }, 201)
  }
  try {
    const created = await executeDeployedWorkflowCommand(profile, [
      "create",
      "Demo",
      "--kind",
      "agent-app",
      "--mode",
      "local-runtime",
      "--slug",
      "demo-app",
      "--region",
      "fsn1",
    ])
    const promoted = await executeDeployedWorkflowCommand(profile, [
      "promote",
      "project-1",
      "environment-1",
      "release-2",
      "--configuration",
      join(root, "configuration.json"),
      "--limits",
      join(root, "limits.json"),
      "--idempotency-key",
      "stable-key",
    ])

    assert.equal(created.footer, "created deployment demo")
    assert.deepEqual(bodies[0], {
      accountId: "account-1",
      name: "Demo",
      kind: "agent_app",
      defaultRuntimeMode: "local_runtime",
      slug: "demo-app",
      defaultRegion: "fsn1",
    })
    assert.deepEqual(bodies[1], {
      accountId: "account-1",
      releaseId: "release-2",
      idempotencyKey: "stable-key",
      configuration: { feature: true },
      limits: { maxConcurrency: 3 },
    })
    assert.equal(promoted.footer, "promotion requested for production")
  } finally {
    globalThis.fetch = originalFetch
    await rm(root, { recursive: true, force: true })
  }
})

test("deployed workflow TUI command changes environment lifecycle through the shared path", async () => {
  const originalFetch = globalThis.fetch
  const calls: Array<{ pathname: string; body: Record<string, unknown> }> = []
  const notices: string[] = []
  const footers: string[] = []
  globalThis.fetch = async (input, init) => {
    calls.push({
      pathname: new URL(String(input)).pathname,
      body: JSON.parse(String(init?.body)) as Record<string, unknown>,
    })
    return jsonResponse({ environment: { ...projectState().environments[0], desiredState: "stopped" } }, 202)
  }
  try {
    const handled = await handleDeployedWorkflowCloudCommand({
      appendNotice: (message) => notices.push(message),
      flashFooter: (message) => footers.push(message),
    }, profile, "deployments", "stop", [
      "project-1",
      "environment-1",
      "--idempotency-key",
      "stable-stop",
    ])

    assert.equal(handled, true)
    assert.deepEqual(calls, [{
      pathname: "/deployment-projects/project-1/environments/environment-1/stop",
      body: { accountId: "account-1", idempotencyKey: "stable-stop" },
    }])
    assert.match(notices[0] ?? "", /state desired=stopped observed=degraded/)
    assert.equal(footers[0], "stop requested for production")
  } finally {
    globalThis.fetch = originalFetch
  }
})

const profile: RelayCloudProfile = {
  apiUrl: "https://cloud.example.test",
  email: "user@example.test",
  accountId: "account-1",
  userId: "user-1",
  accountSlug: "account",
  realmId: "realm-1",
  relayUrl: "wss://relay.example.test",
  issuerId: "issuer-1",
  cloudSessionToken: "session-token",
}

function portfolioItem(): DeploymentPortfolioItem {
  return {
    project: projectState().project,
    defaultEnvironment: projectState().environments[0]!,
    latestRelease: projectState().releases[0]!,
    latestPromotion: projectState().promotions[0]!,
    needsAttention: true,
  }
}

function projectState() {
  return {
    project: {
      id: "project-1",
      accountId: "account-1",
      name: "Demo app",
      slug: "demo",
      kind: "agent_app" as const,
      origin: "native",
      defaultEnvironmentSlug: "production",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
    },
    releases: [{
      id: "release-2",
      projectId: "project-1",
      sequence: 2,
      status: "available",
      packageId: `sha256:${"a".repeat(64)}`,
      packageDigest: `sha256:${"b".repeat(64)}`,
      packageVersion: 3,
      contractVersion: 1,
      verifiedAt: "2026-01-01T00:00:00.000Z",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
    }],
    environments: [{
      id: "environment-1",
      projectId: "project-1",
      name: "Production",
      slug: "production",
      tier: "production",
      runtimeMode: "hosted_container" as const,
      desiredState: "live",
      observedState: "degraded",
      desiredReleaseId: "release-2",
      observedReleaseId: "release-1",
      desiredRevision: 2,
      observedRevision: 1,
      publicUrl: "https://demo.example.test/",
      lastError: "health check failed",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
    }],
    promotions: [{
      id: "promotion-2",
      projectId: "project-1",
      environmentId: "environment-1",
      fromReleaseId: "release-1",
      toReleaseId: "release-2",
      desiredRevision: 2,
      status: "failed",
      requestedAt: "2026-01-01T00:00:00.000Z",
    }],
  }
}

function claimSummary() {
  return {
    id: "claim-1",
    sourceAccountId: "account-1",
    sourceProjectId: "project-1",
    sourceReleaseId: "release-2",
    sourceProjectName: "Demo app",
    sourceProjectSlug: "demo",
    sourceReleaseSequence: 2,
    sourcePackageDigest: `sha256:${"b".repeat(64)}`,
    createdByUserId: "user-1",
    targetAccountId: "customer-account",
    targetEmail: "owner@customer.test",
    ownershipMode: "customer_owned",
    builderRole: "viewer",
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
    members: [{
      id: "member-1",
      projectId: "project-1",
      granteeAccountId: "support-account",
      userId: "support-user",
      userEmail: "support@example.test",
      role: "operator",
      status: "active",
      grantedByUserId: "user-1",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
    }],
  }
}

function promotionResult() {
  const state = projectState()
  return { promotion: state.promotions[0], environment: { ...state.environments[0], observedState: "deploying" } }
}

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } })
}
