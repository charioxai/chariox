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
    "role=owner",
    "capabilities=read,release,operate,manage",
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

test("deployed workflow TUI command drives destination credentials without exposing runtime references", async () => {
  const originalFetch = globalThis.fetch
  const calls: Array<{ readonly pathname: string; readonly body: Record<string, unknown> | null }> = []
  const notices: string[] = []
  globalThis.fetch = async (input, init) => {
    const pathname = new URL(String(input)).pathname
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    calls.push({ pathname, body })
    if (pathname.endsWith("/credentials") || pathname.includes("/credential-bindings")) {
      return jsonResponse({ credentials: credentialState() })
    }
    if (pathname === "/deployment-credentials" && !init?.method) {
      return jsonResponse({ profiles: [{ ...credentialProfile(), runtimeRef: "runtime-ref-secret" }] })
    }
    const operation = pathname === "/deployment-credentials" ? "connect" : pathname.split("/").at(-1)
    return jsonResponse({
      profile: { ...credentialProfile(), runtimeRef: "runtime-ref-secret" },
      job: {
        id: `job-${operation}`,
        type: operation,
        status: "pending",
        runtimeRef: "runtime-ref-secret",
      },
    }, 202)
  }
  try {
    notices.push((await executeDeployedWorkflowCommand(profile, ["credentials", "list"])).notice)
    notices.push((await executeDeployedWorkflowCommand(profile, [
      "credentials", "show", "project/one", "environment/one", "release/one",
    ])).notice)
    await executeDeployedWorkflowCommand(profile, [
      "credentials", "connect", "provider", "codex", "Production Codex",
    ])
    await executeDeployedWorkflowCommand(profile, [
      "credentials", "connect", "integration", "slack", "Customer Slack",
    ])
    for (const operation of ["test", "rotate", "revoke", "purge"] as const) {
      notices.push((await executeDeployedWorkflowCommand(profile, [
        "credentials", operation, "profile/one",
      ])).notice)
    }
    await executeDeployedWorkflowCommand(profile, [
      "credentials", "bind", "project/one", "environment/one", "release/one",
      "provider:codex", "profile/one",
    ])
    await executeDeployedWorkflowCommand(profile, [
      "credentials", "unbind", "project/one", "environment/one", "provider:codex",
    ])

    assert.equal(calls.length, 10)
    assert.deepEqual(calls.slice(0, 4).map((call) => call.pathname), [
      "/deployment-credentials",
      "/deployment-projects/project%2Fone/environments/environment%2Fone/credentials",
      "/deployment-credentials",
      "/deployment-credentials",
    ])
    assert.deepEqual(calls[2]?.body, {
      accountId: "account-1",
      kind: "provider",
      provider: "codex",
      label: "Production Codex",
    })
    assert.deepEqual(calls[3]?.body, {
      accountId: "account-1",
      kind: "integration",
      integration: "slack",
      label: "Customer Slack",
    })
    assert.deepEqual(calls[8]?.body, {
      accountId: "account-1",
      releaseId: "release/one",
      slotId: "provider:codex",
      profileId: "profile/one",
    })
    assert.deepEqual(calls[9]?.body, { accountId: "account-1", slotId: "provider:codex" })
    assert.match(notices[0] ?? "", /credential profile-1 ready/)
    assert.match(notices[1] ?? "", /slot provider:codex ready/)
    assert.equal(notices.some((notice) => notice.includes("runtime-ref-secret")), false)
    await assert.rejects(
      executeDeployedWorkflowCommand(profile, ["credentials", "connect", "provider", "unknown", "Invalid"]),
      /must be codex, claude, opencode, or dev-stub/,
    )
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow TUI command drives the complete domain lifecycle", async () => {
  const originalFetch = globalThis.fetch
  const calls: Array<{
    readonly method: string
    readonly pathname: string
    readonly search: string
    readonly body: Record<string, unknown> | null
  }> = []
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    calls.push({
      method: init?.method ?? "GET",
      pathname: url.pathname,
      search: url.search,
      body: typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null,
    })
    return jsonResponse({ domains: domainState() })
  }
  try {
    const shown = await executeDeployedWorkflowCommand(profile, [
      "domains", "show", "project/one", "environment/one",
    ])
    const added = await executeDeployedWorkflowCommand(profile, [
      "domains", "add", "project/one", "environment/one", "agents.customer.test",
    ])
    for (const operation of ["verify", "canonical", "remove"] as const) {
      await executeDeployedWorkflowCommand(profile, [
        "domains", operation, "project/one", "environment/one", "domain/one",
      ])
    }

    const base = "/deployment-projects/project%2Fone/environments/environment%2Fone/domains"
    assert.deepEqual(calls, [
      { method: "GET", pathname: base, search: "?accountId=account-1", body: null },
      { method: "POST", pathname: base, search: "", body: { accountId: "account-1", hostname: "agents.customer.test" } },
      { method: "POST", pathname: `${base}/domain%2Fone/verify`, search: "", body: { accountId: "account-1" } },
      { method: "POST", pathname: `${base}/domain%2Fone/canonical`, search: "", body: { accountId: "account-1" } },
      { method: "POST", pathname: `${base}/domain%2Fone/remove`, search: "", body: { accountId: "account-1" } },
    ])
    assert.match(shown.notice, /canonical demo\.apps\.example\.test/)
    assert.match(shown.notice, /domain domain-1 custom pending_dns canonical=no/)
    assert.match(shown.notice, /txt_value arroba-domain-token/)
    assert.match(shown.notice, /dns pending/)
    assert.match(shown.notice, /tls pending/)
    assert.equal(shown.footer, "2 deployment domains")
    assert.equal(added.footer, "deployment domain agents.customer.test added")
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow TUI command keeps runtime usage and limits in control-plane sync", async () => {
  const originalFetch = globalThis.fetch
  const root = await mkdtemp(join(tmpdir(), "arroba-deployment-limits-command-"))
  const calls: Array<{
    readonly method: string
    readonly pathname: string
    readonly body: Record<string, unknown> | null
  }> = []
  const limitsPath = join(root, "limits.json")
  await writeFile(limitsPath, JSON.stringify({ concurrency: 4, queue: 12, duration_ms: 20_000 }))
  let limits = { concurrency: 2, queue: 8, duration_ms: 30_000 }
  globalThis.fetch = async (input, init) => {
    const pathname = new URL(String(input)).pathname
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    calls.push({ method: init?.method ?? "GET", pathname, body })
    if (init?.method === "POST") {
      limits = body?.limits as typeof limits
      return jsonResponse({
        environment: { ...projectState().environments[0], limits, desiredRevision: 3 },
        changed: true,
        restartRequested: true,
      })
    }
    return jsonResponse({ usage: runtimeUsage(limits) })
  }
  try {
    const shown = await executeDeployedWorkflowCommand(profile, [
      "usage", "project/one", "environment/one",
    ])
    const limitsShown = await executeDeployedWorkflowCommand(profile, [
      "limits", "show", "project/one", "environment/one",
    ])
    const updated = await executeDeployedWorkflowCommand(profile, [
      "limits", "set", "project/one", "environment/one", limitsPath,
      "--idempotency-key", "limits-1",
    ])

    assert.match(shown.notice, /usage active=1 minute=2 today=4 units=4/)
    assert.match(shown.notice, /invocation invocation-1 completed succeeded/)
    assert.doesNotMatch(shown.notice, /caller-key-secret/)
    assert.match(limitsShown.notice, /limits concurrency=2 queue=8 duration_ms=30000/)
    assert.match(updated.notice, /limits concurrency=4 queue=12 duration_ms=20000/)
    assert.equal(updated.footer, "runtime limits saved; restart requested")
    const base = "/deployment-projects/project%2Fone/environments/environment%2Fone"
    assert.deepEqual(calls.map((call) => [call.method, call.pathname]), [
      ["GET", `${base}/usage`],
      ["GET", `${base}/usage`],
      ["POST", `${base}/limits`],
      ["GET", `${base}/usage`],
    ])
    assert.deepEqual(calls[2]?.body, {
      accountId: "account-1",
      idempotencyKey: "limits-1",
      limits: { concurrency: 4, queue: 12, duration_ms: 20_000 },
    })

    await writeFile(limitsPath, JSON.stringify({ concurency: 4 }))
    await assert.rejects(
      executeDeployedWorkflowCommand(profile, [
        "limits", "set", "project/one", "environment/one", limitsPath,
      ]),
      /unsupported field concurency/,
    )
  } finally {
    globalThis.fetch = originalFetch
    await rm(root, { recursive: true, force: true })
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
  await writeFile(join(root, "limits.json"), JSON.stringify({ concurrency: 3 }))
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
      limits: { concurrency: 3 },
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
    control: {
      role: "owner",
      source: "account",
      canRead: true,
      canRelease: true,
      canOperate: true,
      canManage: true,
    },
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
    lastCheckedAt: "2026-01-01T00:00:00.000Z",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }
}

function credentialState() {
  return {
    projectId: "project-1",
    environmentId: "environment-1",
    releaseId: "release-2",
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

function domainState() {
  return {
    projectId: "project/one",
    environmentId: "environment/one",
    canonicalHostname: "demo.apps.example.test",
    domains: [{
      id: "domain-default",
      accountId: "account-1",
      projectId: "project/one",
      environmentId: "environment/one",
      kind: "default",
      hostname: "demo.apps.example.test",
      publicUrl: "https://demo.apps.example.test",
      status: "ready",
      dnsStatus: "not_required",
      tlsStatus: "ready",
      isCanonical: true,
      redirectToCanonical: false,
      activatedAt: "2026-01-01T00:00:00.000Z",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
    }, {
      id: "domain-1",
      accountId: "account-1",
      projectId: "project/one",
      environmentId: "environment/one",
      kind: "custom",
      hostname: "agents.customer.test",
      publicUrl: "https://agents.customer.test",
      status: "pending_dns",
      dnsStatus: "pending",
      tlsStatus: "pending",
      isCanonical: false,
      redirectToCanonical: true,
      verificationName: "_arroba-verification.agents.customer.test",
      verificationValue: "arroba-domain-token",
      cnameTarget: "ingress.apps.example.test",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
    }],
  }
}

function runtimeUsage(limits: { readonly concurrency: number; readonly queue: number; readonly duration_ms: number }) {
  return {
    accountId: "account-1",
    projectId: "project/one",
    environmentId: "environment/one",
    deploymentId: "deployment-1",
    generatedAt: "2026-01-01T00:00:01.000Z",
    dayStartedAt: "2026-01-01T00:00:00.000Z",
    limits,
    activeInvocations: 1,
    invocationsLastMinute: 2,
    invocationsToday: 4,
    usageUnitsToday: 4,
    succeededToday: 3,
    failedToday: 1,
    timedOutToday: 0,
    interruptedToday: 0,
    averageDurationMs: 240,
    maximumDurationMs: 400,
    averageQueuedMs: 8,
    requestBytesToday: 1_024,
    responseBytesToday: 2_048,
    recentInvocations: [{
      invocationId: "invocation-1",
      callerKeyHash: "caller-key-secret",
      transport: "http",
      state: "completed",
      outcome: "succeeded",
      statusCode: 200,
      errorCode: null,
      queuedMs: 8,
      durationMs: 240,
      requestBytes: 100,
      responseBytes: 200,
      usageUnits: 1,
      startedAt: "2026-01-01T00:00:00.000Z",
      finishedAt: "2026-01-01T00:00:00.240Z",
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
