import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import {
  executeDeployedWorkflowCommand,
  formatDeploymentCredentialEnrollment,
  formatDeploymentCredentialProfile,
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
    if (pathname.endsWith("/enrollment")) {
      return jsonResponse({
        enrollment: credentialEnrollment(decodeURIComponent(pathname.split("/").at(-2) ?? "profile-1")),
      })
    }
    if (pathname === "/deployment-credentials" && !init?.method) {
      return jsonResponse({ profiles: [
        {
          ...credentialProfile(),
          runtimeRef: "runtime-ref-secret",
          runnerId: "runner-id-secret",
          token: "token-secret",
          sourcePath: "/private/credential/source",
        },
        { ...credentialProfile(), id: "profile/one", label: "Encoded Codex" },
        {
          ...credentialProfile(),
          id: "profile-retry",
          label: "Retry Codex",
          status: "error",
          verification: "expired",
          enrollment: {
            ...credentialEnrollment("profile-retry"),
            status: "expired",
            verificationUrl: null,
            userCode: null,
          },
        },
        { ...credentialProfile(), id: "profile-retired", label: "Retired Codex", status: "revoked" },
      ], setupAccess: "available" })
    }
    const operation = pathname === "/deployment-credentials" || pathname.endsWith("/setup")
      ? "connect"
      : pathname.split("/").at(-1)
    return jsonResponse({
      profile: {
        ...credentialProfile(),
        ...(operation === "connect" || operation === "rotate" || operation === "retry" ? {
          status: "connecting",
          verification: "setup_required",
          enrollment: { ...credentialEnrollment("profile-1"), verificationUrl: null, userCode: null },
        } : {}),
        ...(operation === "purge" ? { id: "profile-retired", status: "revoked" } : {}),
        runtimeRef: "runtime-ref-secret",
        runnerId: "runner-id-secret",
        token: "token-secret",
        sourcePath: "/private/credential/source",
      },
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
    notices.push((await executeDeployedWorkflowCommand(profile, [
      "credentials", "setup", "provider", "codex", "Production Codex",
    ])).notice)
    notices.push((await executeDeployedWorkflowCommand(profile, [
      "credentials", "connect", "integration", "slack", "Customer Slack",
    ])).notice)
    notices.push((await executeDeployedWorkflowCommand(profile, [
      "credentials", "enrollment", "profile/one",
    ])).notice)
    notices.push((await executeDeployedWorkflowCommand(profile, [
      "credentials", "retry", "profile-retry",
    ])).notice)
    notices.push((await executeDeployedWorkflowCommand(profile, [
      "credentials", "setup", "profile-retry",
    ])).notice)
    for (const operation of ["test", "rotate", "revoke"] as const) {
      notices.push((await executeDeployedWorkflowCommand(profile, [
        "credentials", operation, "profile/one",
      ])).notice)
    }
    notices.push((await executeDeployedWorkflowCommand(profile, [
      "credentials", "purge", "profile-retired",
    ])).notice)
    await executeDeployedWorkflowCommand(profile, [
      "credentials", "bind", "project/one", "environment/one", "release/one",
      "provider:codex", "profile/one",
    ])
    await executeDeployedWorkflowCommand(profile, [
      "credentials", "unbind", "project/one", "environment/one", "provider:codex",
    ])

    const setupCalls = calls.filter((call) => call.pathname === "/deployment-credentials" && call.body)
    assert.equal(setupCalls.length, 2)
    assert.deepEqual(setupCalls[0]?.body, {
      accountId: "account-1",
      kind: "provider",
      provider: "codex",
      label: "Production Codex",
    })
    assert.deepEqual(setupCalls[1]?.body, {
      accountId: "account-1",
      kind: "integration",
      integration: "slack",
      label: "Customer Slack",
    })
    assert.equal(setupCalls.some((call) => "enrollmentMode" in (call.body ?? {})), false)
    assert.ok(calls.some((call) => call.pathname === "/deployment-credentials/profile%2Fone/enrollment"))
    const rotation = calls.find((call) => call.pathname.endsWith("/rotate"))
    assert.equal("enrollmentMode" in (rotation?.body ?? {}), false)
    const retries = calls.filter((call) => call.pathname.endsWith("/setup"))
    assert.equal(retries.length, 2)
    assert.equal(retries.some((call) => "enrollmentMode" in (call.body ?? {})), false)
    const binding = calls.find((call) => call.pathname.endsWith("/credential-bindings"))
    assert.deepEqual(binding?.body, {
      accountId: "account-1",
      releaseId: "release/one",
      slotId: "provider:codex",
      profileId: "profile/one",
    })
    const unbinding = calls.find((call) => call.pathname.endsWith("/credential-bindings/revoke"))
    assert.deepEqual(unbinding?.body, { accountId: "account-1", slotId: "provider:codex" })
    assert.match(notices[0] ?? "", /credential profile-1 ready/)
    assert.match(notices[1] ?? "", /slot provider:codex ready/)
    assert.ok(notices.some((notice) => notice.includes("verification_url https://auth.openai.com/codex/device?user_code=ABCD-1234")))
    assert.ok(notices.some((notice) => notice.includes("user_code ABCD-1234")))
    assert.equal(
      notices.some((notice) => /runtime-ref-secret|runner-id-secret|token-secret|private\/credential\/source/.test(notice)),
      false,
    )
    await assert.rejects(
      executeDeployedWorkflowCommand(profile, ["credentials", "setup", "provider", "unknown", "Invalid"]),
      /must be codex, claude, opencode, or dev-stub/,
    )
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployment credential formatting preserves provider auth URLs and forces runner-seeded unverified", () => {
  const runnerSeeded = formatDeploymentCredentialProfile({
    ...credentialProfile(),
    verification: "verified",
    accountLabel: "misleading@example.test",
    enrollment: {
      ...credentialEnrollment("profile-1"),
      mode: "runner_seeded",
      status: "consumed",
      verificationUrl: null,
      userCode: null,
    },
  })
  assert.match(runnerSeeded, /verification unverified/)
  assert.match(runnerSeeded, /account unverified/)
  assert.match(runnerSeeded, /actions test rotate revoke/)
  assert.doesNotMatch(runnerSeeded, /retry_setup/)
  assert.doesNotMatch(runnerSeeded, /misleading@example\.test|verification verified/)

  const retryable = formatDeploymentCredentialProfile({
    ...credentialProfile(),
    status: "expired",
    verification: "unverified",
    enrollment: {
      ...credentialEnrollment("profile-1"),
      mode: "runner_seeded",
      status: "expired",
      verificationUrl: null,
      userCode: null,
    },
  })
  assert.match(retryable, /verification expired/)
  assert.match(retryable, /actions retry_setup/)
  assert.doesNotMatch(retryable, /actions test rotate revoke/)

  const failedRunnerSeeded = formatDeploymentCredentialProfile({
    ...credentialProfile(),
    status: "error",
    verification: "unverified",
    enrollment: {
      ...credentialEnrollment("profile-1"),
      mode: "runner_seeded",
      status: "failed",
      verificationUrl: null,
      userCode: null,
    },
  })
  assert.match(failedRunnerSeeded, /verification failed/)
  assert.match(failedRunnerSeeded, /actions retry_setup/)
  assert.doesNotMatch(failedRunnerSeeded, /actions test rotate revoke/)

  const claudeProfile = {
    ...credentialProfile(),
    id: "profile-claude",
    provider: "claude",
  }
  const pkceUrl = "https://claude.com/cai/oauth/authorize"
    + "?response_type=code"
    + "&client_id=claude-code"
    + "&redirect_uri=https%3A%2F%2Fclaude.com%2Fcai%2Foauth%2Fcode%2Fcallback"
    + "&scope=org%3Acreate_api_key%20user%3Aprofile"
    + "&code_challenge=Challenge-1234"
    + "&code_challenge_method=S256"
    + "&state=State-1234"
  const pkce = formatDeploymentCredentialEnrollment({
    ...credentialEnrollment("profile-claude"),
    verificationUrl: pkceUrl,
    userCode: null,
  }, claudeProfile)
  assert.match(pkce, new RegExp(`verification_url ${escapeRegex(pkceUrl)}`))

  const unsafe = formatDeploymentCredentialEnrollment({
    ...credentialEnrollment("profile-unsafe"),
    verificationUrl: `${pkceUrl}&access_token=secret-token`,
    userCode: null,
  }, claudeProfile)
  assert.doesNotMatch(unsafe, /verification_url|secret-token/)

  for (const unsafeQuery of ["code-verifier=secret-verifier", "device.code=secret-device"]) {
    const unsafeNormalizedName = formatDeploymentCredentialEnrollment({
      ...credentialEnrollment("profile-unsafe"),
      verificationUrl: `${pkceUrl}&${unsafeQuery}`,
      userCode: null,
    }, claudeProfile)
    assert.doesNotMatch(unsafeNormalizedName, /verification_url|secret-/)
  }

  for (const unsafeQuery of [
    "unexpected=safe-looking-value",
    "state=Duplicate-State",
    "code=callback-secret",
    "code_challenge_method=plain",
  ]) {
    const unsafePolicyParameter = formatDeploymentCredentialEnrollment({
      ...credentialEnrollment("profile-unsafe-policy"),
      verificationUrl: `${pkceUrl}&${unsafeQuery}`,
      userCode: null,
    }, claudeProfile)
    assert.doesNotMatch(unsafePolicyParameter, /verification_url|callback-secret/)
  }

  const excessiveParameters = formatDeploymentCredentialEnrollment({
    ...credentialEnrollment("profile-excessive-parameters"),
    verificationUrl: `${pkceUrl}&${Array.from({ length: 25 }, (_, index) => `state_${index}=safe`).join("&")}`,
    userCode: null,
  }, claudeProfile)
  assert.doesNotMatch(excessiveParameters, /verification_url/)

  const excessiveQuery = formatDeploymentCredentialEnrollment({
    ...credentialEnrollment("profile-excessive-query"),
    verificationUrl: `${pkceUrl}&state=${"a".repeat(1_536)}`,
    userCode: null,
  }, claudeProfile)
  assert.doesNotMatch(excessiveQuery, /verification_url/)
})

test("deployed workflow TUI blocks credential operations while setup is active", async () => {
  const originalFetch = globalThis.fetch
  const methods: string[] = []
  globalThis.fetch = async (_input, init) => {
    methods.push(init?.method ?? "GET")
    return jsonResponse({ profiles: [{
      ...credentialProfile(),
      verification: "setup_in_progress",
      enrollment: { ...credentialEnrollment("profile-1"), status: "claimed" },
    }], setupAccess: "available" })
  }
  try {
    await assert.rejects(
      () => executeDeployedWorkflowCommand(profile, ["credentials", "rotate", "profile-1"]),
      /rotate is unavailable until setup finishes/,
    )
    assert.deepEqual(methods, ["GET"])
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("explicit TUI enrollment command reports member authorization denial", async () => {
  const originalFetch = globalThis.fetch
  globalThis.fetch = async () => jsonResponse({
    error: { message: "Credential enrollment setup requires account owner or admin access" },
  }, 403)
  try {
    await assert.rejects(
      () => executeDeployedWorkflowCommand(profile, ["credentials", "enrollment", "profile-1"]),
      /requires account owner or admin access/,
    )
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow TUI advertises only safe credential actions to members", async () => {
  const originalFetch = globalThis.fetch
  globalThis.fetch = async () => jsonResponse({
    profiles: [
      credentialProfile(),
      { ...credentialProfile(), id: "profile-revoked", status: "revoked" },
    ],
    setupAccess: "restricted",
  })
  try {
    const output = await executeDeployedWorkflowCommand(profile, ["credentials", "list"])
    assert.match(output.notice, /credential profile-1 ready[\s\S]*actions test/)
    assert.match(output.notice, /credential profile-revoked revoked[\s\S]*actions none/)
    assert.doesNotMatch(output.notice, /actions (?:retry_setup|test rotate revoke|purge)/)
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("TUI setup preserves a successful mutation when privileged details are unavailable", async () => {
  const originalFetch = globalThis.fetch
  let request = 0
  globalThis.fetch = async () => {
    request += 1
    if (request === 1) {
      return jsonResponse({ profiles: [], setupAccess: "available" })
    }
    if (request === 2) {
      return jsonResponse({
        profile: {
          ...credentialProfile(),
          status: "connecting",
          verification: "setup_required",
          enrollment: { ...credentialEnrollment("profile-1"), verificationUrl: null, userCode: null },
        },
        job: { id: "job-connect", type: "connect", status: "pending" },
      }, 201)
    }
    return jsonResponse({ error: { message: "setup detail service unavailable" } }, 503)
  }
  try {
    const output = await executeDeployedWorkflowCommand(profile, [
      "credentials", "setup", "provider", "codex", "Production Codex",
    ])
    assert.match(output.notice, /credential profile-1 connecting/)
    assert.match(output.notice, /setup_details unavailable/)
    assert.match(output.footer, /setup details unavailable/)
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
    assert.match(shown.notice, /active_alerts 1/)
    assert.match(shown.notice, /alert error_rate severity=warning current=25 threshold=20 unit=percent/)
    assert.match(shown.notice, /diagnostic runtime_process healthy/)
    assert.match(shown.notice, /audit deployment_environment\.limits_updated actor=user-1/)
    assert.match(shown.notice, /privacy capture=metadata_only content_capture=disabled active_invocations_protected=yes state_contract=stateless_external_storage/)
    assert.match(shown.notice, /invocation invocation-1 completed succeeded/)
    assert.match(shown.notice, /admission=deferred_replay/)
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

test("deployed workflow TUI command keeps emergency admission and retention policy in sync", async () => {
  const originalFetch = globalThis.fetch
  const root = await mkdtemp(join(tmpdir(), "arroba-deployment-operations-command-"))
  const policyPath = join(root, "operations.json")
  const calls: Array<{
    readonly method: string
    readonly pathname: string
    readonly body: Record<string, unknown> | null
  }> = []
  const savedPolicy = operationsPolicy({
    invocationMetadataRetentionDays: 7,
    deploymentLogRetentionDays: 14,
  })
  await writeFile(policyPath, JSON.stringify(savedPolicy))
  let policy = operationsPolicy()
  globalThis.fetch = async (input, init) => {
    const pathname = new URL(String(input)).pathname
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    calls.push({ method: init?.method ?? "GET", pathname, body })
    if (init?.method === "POST") {
      policy = body?.policy as typeof policy
      return jsonResponse({
        environment: { ...projectState().environments[0], operationsPolicy: policy, operationsPolicyVersion: 2 },
        changed: true,
        prunedInvocationCount: policy.invocationMetadataRetentionDays === 7 ? 2 : 0,
        prunedLogCount: policy.deploymentLogRetentionDays === 14 ? 1 : 0,
      })
    }
    return jsonResponse({ usage: { ...runtimeUsage({ concurrency: 2, queue: 8, duration_ms: 30_000 }), operationsPolicy: policy, operationsPolicyVersion: 2 } })
  }
  try {
    const shown = await executeDeployedWorkflowCommand(profile, [
      "operations", "show", "project/one", "environment/one",
    ])
    const denied = await executeDeployedWorkflowCommand(profile, [
      "operations", "deny", "project/one", "environment/one",
      "--reason", "provider incident", "--idempotency-key", "operations-deny-1",
    ])
    const resumed = await executeDeployedWorkflowCommand(profile, [
      "operations", "resume", "project/one", "environment/one",
      "--idempotency-key", "operations-resume-1",
    ])
    const saved = await executeDeployedWorkflowCommand(profile, [
      "operations", "set", "project/one", "environment/one", policyPath,
      "--idempotency-key", "operations-set-1",
    ])

    assert.match(shown.notice, /operations admission=accepting policy_version=2 reason=none/)
    assert.match(shown.notice, /content_capture=disabled/)
    assert.match(denied.notice, /admission=denied .*reason=provider incident/)
    assert.equal(denied.footer, "new deployment calls denied; in-flight calls continue")
    assert.match(resumed.notice, /admission=accepting .*reason=none/)
    assert.equal(resumed.footer, "deployment calls resumed")
    assert.match(saved.notice, /invocation_metadata_days=7 deployment_log_days=14/)
    assert.equal(saved.footer, "operations policy saved; 3 expired records removed")
    const postBodies = calls.filter((call) => call.method === "POST").map((call) => call.body)
    assert.equal((postBodies[0]?.policy as { admissionMode: string }).admissionMode, "denied")
    assert.equal((postBodies[0]?.policy as { admissionReason: string }).admissionReason, "provider incident")
    assert.equal(postBodies[0]?.idempotencyKey, "operations-deny-1")
    assert.equal((postBodies[1]?.policy as { admissionMode: string }).admissionMode, "accepting")
    assert.equal(postBodies[1]?.idempotencyKey, "operations-resume-1")
    assert.deepEqual(postBodies[2]?.policy, savedPolicy)
    assert.equal(postBodies[2]?.idempotencyKey, "operations-set-1")
  } finally {
    globalThis.fetch = originalFetch
    await rm(root, { recursive: true, force: true })
  }
})

test("deployed workflow TUI command exports verified telemetry and deletes retained metadata", async () => {
  const originalFetch = globalThis.fetch
  const root = await mkdtemp(join(tmpdir(), "arroba-deployment-telemetry-command-"))
  const outputPath = join(root, "telemetry.json")
  const corruptPath = join(root, "corrupt.json")
  const content = Buffer.from("{\"schemaVersion\":1,\"records\":[]}\n", "utf8")
  const sha256 = `sha256:${createHash("sha256").update(content).digest("hex")}`
  const calls: Array<{
    readonly method: string
    readonly pathname: string
    readonly body: Record<string, unknown> | null
  }> = []
  let corrupt = false
  globalThis.fetch = async (input, init) => {
    const pathname = new URL(String(input)).pathname
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    calls.push({ method: init?.method ?? "GET", pathname, body })
    if (pathname.endsWith("/telemetry/delete")) {
      return jsonResponse({
        deletedAt: "2026-07-13T12:00:00.000Z",
        deletedInvocationCount: 2,
        deletedLogCount: 3,
        protectedActiveInvocationCount: 1,
      })
    }
    return jsonResponse({
      exportId: "export-1",
      filename: "server-name.json",
      mediaType: "application/json",
      generatedAt: "2026-07-13T11:00:00.000Z",
      byteSize: content.byteLength,
      sha256: corrupt ? `sha256:${"0".repeat(64)}` : sha256,
      contentBase64: content.toString("base64"),
      counts: { invocationMetadata: 2, deploymentLogs: 3, auditEvents: 4 },
    })
  }
  try {
    const exported = await executeDeployedWorkflowCommand(profile, [
      "telemetry", "export", "project/one", "environment/one", outputPath,
    ])
    assert.equal(await readFile(outputPath, "utf8"), content.toString("utf8"))
    assert.equal((await stat(outputPath)).mode & 0o777, 0o600)
    assert.match(exported.notice, /telemetry_export export-1/)
    assert.match(exported.notice, /invocation_metadata 2/)
    assert.equal(exported.footer, `deployment telemetry exported to ${outputPath}`)

    const deleted = await executeDeployedWorkflowCommand(profile, [
      "telemetry", "delete", "project/one", "environment/one",
      "--idempotency-key", "telemetry-delete-1",
    ])
    assert.match(deleted.notice, /active_invocations_protected 1/)
    assert.equal(deleted.footer, "5 retained metadata records deleted")

    const base = "/deployment-projects/project%2Fone/environments/environment%2Fone/telemetry"
    assert.deepEqual(calls.slice(0, 2).map((call) => [call.method, call.pathname]), [
      ["POST", `${base}/export`],
      ["POST", `${base}/delete`],
    ])
    assert.deepEqual(calls[0]?.body, { accountId: "account-1" })
    assert.deepEqual(calls[1]?.body, {
      accountId: "account-1",
      idempotencyKey: "telemetry-delete-1",
    })

    corrupt = true
    await assert.rejects(
      executeDeployedWorkflowCommand(profile, [
        "telemetry", "export", "project/one", "environment/one", corruptPath,
      ]),
      /digest does not match/,
    )
    await assert.rejects(readFile(corruptPath), /ENOENT/)
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
    kind: "provider" as const,
    provider: "codex",
    label: "Production Codex",
    accountLabel: "customer@example.test",
    version: 2,
    status: "ready" as const,
    runnerConnected: true,
    lastCheckedAt: "2026-01-01T00:00:00.000Z",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }
}

function credentialEnrollment(profileId: string) {
  return {
    id: `enrollment-${profileId}`,
    profileId,
    targetVersion: 3,
    mode: "provider_native" as const,
    status: "claimed" as const,
    instructions: "Open the provider verification page.",
    verificationUrl: "https://auth.openai.com/codex/device?user_code=ABCD-1234",
    userCode: "ABCD-1234",
    expiresAt: "2026-07-15T12:30:00.000Z",
    createdAt: "2026-07-15T12:00:00.000Z",
    updatedAt: "2026-07-15T12:01:00.000Z",
  }
}

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
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
    alerts: [{
      code: "error_rate",
      severity: "warning" as const,
      message: "Invocation error rate is 25%",
      observedAt: "2026-01-01T00:00:01.000Z",
      currentValue: 25,
      threshold: 20,
      unit: "percent",
    }],
    diagnostics: [{
      code: "runtime_process",
      status: "healthy" as const,
      message: "Runtime is ready with 1 ready replica",
      observedAt: "2026-01-01T00:00:01.000Z",
      details: { deployment_id: "deployment-1", queue_depth: 0 },
    }],
    auditEvents: [{
      auditEventId: "audit-1",
      actorUserId: "user-1",
      actorKind: "USER" as const,
      eventType: "deployment_environment.limits_updated",
      subjectType: "deployment_environment",
      subjectId: "environment/one",
      occurredAt: "2026-01-01T00:00:01.000Z",
    }],
    privacy: {
      captureMode: "metadata_only" as const,
      contentCaptureEnabled: false as const,
      invocationMetadataRetentionDays: 30,
      deploymentLogRetentionDays: 30,
      activeInvocationsProtectedFromDeletion: true as const,
      stateContract: "stateless_external_storage" as const,
      ephemeralStoragePersistent: false as const,
    },
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
      admissionDeferred: true,
      startedAt: "2026-01-01T00:00:00.000Z",
      finishedAt: "2026-01-01T00:00:00.240Z",
    }],
  }
}

function operationsPolicy(overrides: Partial<{
  readonly admissionMode: "accepting" | "denied"
  readonly admissionReason: string | null
  readonly invocationMetadataRetentionDays: number
  readonly deploymentLogRetentionDays: number
}> = {}) {
  return {
    admissionMode: "accepting" as const,
    admissionReason: null,
    invocationMetadataRetentionDays: 30,
    deploymentLogRetentionDays: 30,
    alertThresholds: {
      errorRatePercent: 20,
      averageDurationMs: 30_000,
      queueDepthPercent: 80,
      dailyUsagePercent: 80,
      healthStaleSeconds: 120,
    },
    ...overrides,
  }
}

function promotionResult() {
  const state = projectState()
  return { promotion: state.promotions[0], environment: { ...state.environments[0], observedState: "deploying" } }
}

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json", "x-request-id": "request-1" },
  })
}
