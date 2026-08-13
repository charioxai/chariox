import assert from "node:assert/strict"
import test from "node:test"

import {
  armClaudeCredentialEnrollment,
  type ClaudeCredentialEnrollmentBinding,
} from "./deployed-workflow-credential-enrollment.js"
import {
  executeDeployedWorkflowCommand,
  handleDeployedWorkflowCloudCommand,
  type DeployedWorkflowCommandRuntime,
} from "./deployed-workflow-command.js"
import type { RuntimeSession } from "./cli-types.js"
import type {
  DeploymentCredentialCallbackChannelResult,
  DeploymentCredentialEnrollmentMode,
  DeploymentCredentialEnrollmentStatus,
  DeploymentCredentialProfileResult,
} from "./deployed-workflow-types.js"
import type { RelayCloudProfile } from "./preferences.js"

test("attached TUI setup arms protocol 241 before Cloud and preserves the shared web/TUI interaction projection", async () => {
  const originalFetch = globalThis.fetch
  const order: string[] = []
  const cloudBodies: Record<string, unknown>[] = []
  const kernelRequests: Record<string, unknown>[] = []
  const notices: string[] = []
  const session = attachedSession()
  const sharedInteraction = session.active_interactions?.[0]
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    if (url.pathname === "/deployment-credentials" && !init?.method) {
      order.push("cloud-list")
      return jsonResponse({ profiles: [], setupAccess: "available" })
    }
    if (url.pathname === "/deployment-credentials") {
      order.push("cloud-create")
      return jsonResponse(credentialMutation(), 201)
    }
    if (url.pathname.endsWith("/enrollment/callback-channel/arm")) {
      order.push("cloud-arm")
      cloudBodies.push(body ?? {})
      return jsonResponse({
        ...callbackArmResult(binding()),
        callback: "must-never-be-rendered-or-stored",
      })
    }
    throw new Error(`unexpected request ${url.pathname}`)
  }
  try {
    const handled = await handleDeployedWorkflowCloudCommand({
      ...attachedRuntime(order, kernelRequests, session),
      appendNotice: (message) => notices.push(message),
      flashFooter: () => {},
    }, cloudProfile, "deployments", "credentials", [
      "setup", "provider", "claude", "Customer Claude",
    ])

    assert.equal(handled, true)
    assert.deepEqual(order, ["cloud-list", "cloud-create", "relay-status", "kernel-arm", "cloud-arm"])
    assert.deepEqual(kernelRequests, [{
      ArmDeploymentCredentialEnrollment: {
        session_id: "session-1",
        attachment_id: "attachment-1",
        agent_id: "agent-1",
        enrollment_id: "enrollment-1",
        profile_id: "profile-claude",
        target_version: 1,
      },
    }])
    assert.deepEqual(cloudBodies, [{
      accountId: "account-1",
      enrollmentId: "enrollment-1",
      targetVersion: 1,
      realmId: "realm-1",
      kernelTarget: "kernel-1",
      sessionId: "session-1",
      agentId: "agent-1",
    }])
    assert.equal(session.active_interactions?.[0], sharedInteraction)
    assert.doesNotMatch(notices.join("\n"), /must-never-be-rendered-or-stored|callback/i)
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("attached TUI rotate binds the new Claude enrollment version", async () => {
  const originalFetch = globalThis.fetch
  const order: string[] = []
  const session = attachedSession()
  const rotated = credentialMutation({ targetVersion: 3, profileVersion: 2 })
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    if (url.pathname === "/deployment-credentials" && !init?.method) {
      order.push("cloud-list")
      return jsonResponse({ profiles: [readyClaudeProfile()], setupAccess: "available" })
    }
    if (url.pathname.endsWith("/rotate")) {
      order.push("cloud-rotate")
      return jsonResponse(rotated, 202)
    }
    if (url.pathname.endsWith("/enrollment/callback-channel/arm")) {
      order.push("cloud-arm")
      return jsonResponse(callbackArmResult(binding({ targetVersion: 3 })))
    }
    throw new Error(`unexpected request ${url.pathname}`)
  }
  try {
    const output = await executeDeployedWorkflowCommand(
      cloudProfile,
      ["credentials", "rotate", "profile-claude"],
      attachedRuntime(order, [], session, { targetVersion: 3 }),
    )
    assert.deepEqual(order, ["cloud-list", "cloud-rotate", "relay-status", "kernel-arm", "cloud-arm"])
    assert.match(output.notice, /version 2/)
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("Claude setup retry reuses one immutable callback binding after a transient Cloud failure", async () => {
  const originalFetch = globalThis.fetch
  const callbackBodies: Record<string, unknown>[] = []
  let callbackAttempts = 0
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    if (url.pathname === "/deployment-credentials" && !init?.method) {
      return jsonResponse({ profiles: [retryableClaudeProfile()], setupAccess: "available" })
    }
    if (url.pathname.endsWith("/setup")) return jsonResponse(credentialMutation(), 202)
    if (url.pathname.endsWith("/enrollment/callback-channel/arm")) {
      callbackBodies.push(body ?? {})
      callbackAttempts += 1
      if (callbackAttempts === 1) {
        return jsonResponse({ error: { message: "temporary callback arm failure" } }, 503)
      }
      return jsonResponse(callbackArmResult(binding()))
    }
    throw new Error(`unexpected request ${url.pathname}`)
  }
  try {
    await executeDeployedWorkflowCommand(
      cloudProfile,
      ["credentials", "retry", "profile-claude"],
      attachedRuntime([], [], attachedSession()),
    )
    assert.equal(callbackAttempts, 2)
    assert.deepEqual(callbackBodies[0], callbackBodies[1])
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("Claude enrollment command idempotently re-arms the same pending callback route", async () => {
  const originalFetch = globalThis.fetch
  const callbackBodies: Record<string, unknown>[] = []
  const kernelRequests: Record<string, unknown>[] = []
  const paths: string[] = []
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    paths.push(url.pathname)
    if (url.pathname === "/deployment-credentials") {
      return jsonResponse({ profiles: [credentialMutation().profile], setupAccess: "available" })
    }
    if (url.pathname.endsWith("/enrollment")) {
      return jsonResponse({ enrollment: credentialMutation().profile.enrollment })
    }
    if (url.pathname.endsWith("/enrollment/callback-channel/arm")) {
      callbackBodies.push(body ?? {})
      return jsonResponse(callbackArmResult(binding()))
    }
    throw new Error(`unexpected request ${url.pathname}`)
  }
  try {
    const runtime = attachedRuntime([], kernelRequests, attachedSession())
    await executeDeployedWorkflowCommand(
      cloudProfile,
      ["credentials", "enrollment", "profile-claude"],
      runtime,
    )
    await executeDeployedWorkflowCommand(
      cloudProfile,
      ["credentials", "enrollment", "profile-claude"],
      runtime,
    )

    assert.equal(kernelRequests.length, 2)
    assert.deepEqual(kernelRequests[0], kernelRequests[1])
    assert.equal(callbackBodies.length, 2)
    assert.deepEqual(callbackBodies[0], callbackBodies[1])
    assert.equal(paths.filter((path) => path === "/deployment-credentials").length, 2)
    assert.equal(paths.filter((path) => path.endsWith("/enrollment")).length, 2)
    assert.equal(paths.some((path) => path.endsWith("/rotate") || path.endsWith("/setup")), false)
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("provider-native Claude setup fails closed without an attached TUI", async () => {
  const originalFetch = globalThis.fetch
  const paths: string[] = []
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    paths.push(url.pathname)
    if (url.pathname === "/deployment-credentials" && !init?.method) {
      return jsonResponse({ profiles: [], setupAccess: "available" })
    }
    if (url.pathname === "/deployment-credentials") return jsonResponse(credentialMutation(), 201)
    throw new Error("callback channel must not be armed")
  }
  try {
    await assert.rejects(
      executeDeployedWorkflowCommand(cloudProfile, [
        "credentials", "setup", "provider", "claude", "Customer Claude",
      ]),
      /requires an attached Chariox TUI/,
    )
    assert.deepEqual(paths, ["/deployment-credentials", "/deployment-credentials"])
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("runner-seeded Claude setup preserves the standalone flow", async () => {
  const originalFetch = globalThis.fetch
  const paths: string[] = []
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    paths.push(url.pathname)
    if (url.pathname === "/deployment-credentials" && !init?.method) {
      return jsonResponse({ profiles: [], setupAccess: "available" })
    }
    if (url.pathname === "/deployment-credentials") {
      return jsonResponse(credentialMutation({ mode: "runner_seeded" }), 201)
    }
    throw new Error(`unexpected request ${url.pathname}`)
  }
  try {
    const output = await executeDeployedWorkflowCommand(cloudProfile, [
      "credentials", "setup", "provider", "claude", "Seeded Claude",
    ])
    assert.match(output.notice, /setup_mode runner_seeded/)
    assert.deepEqual(paths, ["/deployment-credentials", "/deployment-credentials"])
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("old protocol and stale or mismatched kernel arms never open Cloud", async (t) => {
  await t.test("old protocol", async () => {
    let cloudCalls = 0
    await assert.rejects(
      armClaudeCredentialEnrollment({
        sendKernelRequest: async () => {
          throw new Error("unknown variant ArmDeploymentCredentialEnrollment")
        },
        armCloudCallbackChannel: async () => {
          cloudCalls += 1
          return callbackArmResult(binding())
        },
      }, binding()),
      /kernel protocol 241.*unknown variant/,
    )
    assert.equal(cloudCalls, 0)
  })

  await t.test("mismatched response", async () => {
    let cloudCalls = 0
    await assert.rejects(
      armClaudeCredentialEnrollment({
        sendKernelRequest: async () => kernelArmResponse(binding({ sessionId: "other-session" })),
        armCloudCallbackChannel: async () => {
          cloudCalls += 1
          return callbackArmResult(binding())
        },
      }, binding()),
      /mismatched credential enrollment arm \(session_id\)/,
    )
    assert.equal(cloudCalls, 0)
  })

  await t.test("stale response", async () => {
    let cloudCalls = 0
    await assert.rejects(
      armClaudeCredentialEnrollment({
        now: () => 10_000,
        sendKernelRequest: async () => kernelArmResponse(binding(), 9_999),
        armCloudCallbackChannel: async () => {
          cloudCalls += 1
          return callbackArmResult(binding())
        },
      }, binding()),
      /stale credential enrollment arm/,
    )
    assert.equal(cloudCalls, 0)
  })
})

test("mismatched Cloud arm fails after the exact kernel arm", async () => {
  let kernelCalls = 0
  await assert.rejects(
    armClaudeCredentialEnrollment({
      sendKernelRequest: async () => {
        kernelCalls += 1
        return kernelArmResponse(binding())
      },
      armCloudCallbackChannel: async () => callbackArmResult(binding({ agentId: "agent-other" })),
    }, binding()),
    /mismatched credential callback channel \(agentId\)/,
  )
  assert.equal(kernelCalls, 1)
})

function attachedRuntime(
  order: string[],
  kernelRequests: Record<string, unknown>[],
  session: RuntimeSession,
  overrides: Partial<ClaudeCredentialEnrollmentBinding> = {},
): DeployedWorkflowCommandRuntime {
  const expected = binding(overrides)
  return {
    isAttached: () => true,
    sessionState: () => session,
    attachmentState: () => ({ id: "attachment-1", session_id: "session-1" }),
    getRelayStatus: async () => {
      order.push("relay-status")
      return { configured: true, connected: true, daemon_id: "kernel-1" }
    },
    sendCredentialEnrollmentKernelRequest: async (request) => {
      order.push("kernel-arm")
      kernelRequests.push(request)
      return kernelArmResponse(expected)
    },
  }
}

function attachedSession(): RuntimeSession {
  return {
    id: "session-1",
    focused_agent_id: "agent-1",
    agents: [{ id: "agent-1" }],
    active_interactions: [{
      id: "interaction-shared",
      session_id: "session-1",
      agent_id: "agent-1",
      level: "warning",
      message: "Shared kernel interaction",
      choices: [{ id: "cancel", label: "Cancel" }],
    }],
  } as unknown as RuntimeSession
}

function credentialMutation(overrides: {
  readonly mode?: DeploymentCredentialEnrollmentMode
  readonly status?: DeploymentCredentialEnrollmentStatus
  readonly targetVersion?: number
  readonly profileVersion?: number
} = {}): DeploymentCredentialProfileResult {
  return {
    profile: {
      ...readyClaudeProfile(),
      version: overrides.profileVersion ?? 1,
      status: "connecting",
      verification: "setup_required",
      enrollment: {
        id: "enrollment-1",
        profileId: "profile-claude",
        targetVersion: overrides.targetVersion ?? 1,
        mode: overrides.mode ?? "provider_native",
        status: overrides.status ?? "pending",
        expiresAt: "2099-07-15T12:30:00.000Z",
        createdAt: "2026-07-15T12:00:00.000Z",
        updatedAt: "2026-07-15T12:00:00.000Z",
      },
    },
    job: {
      id: "job-1",
      accountId: "account-1",
      profileId: "profile-claude",
      type: "connect",
      status: "pending",
      createdAt: "2026-07-15T12:00:00.000Z",
      updatedAt: "2026-07-15T12:00:00.000Z",
    },
  }
}

function readyClaudeProfile() {
  return {
    id: "profile-claude",
    accountId: "account-1",
    kind: "provider" as const,
    provider: "claude",
    label: "Customer Claude",
    version: 2,
    status: "ready" as const,
    runnerConnected: true,
    createdAt: "2026-07-15T12:00:00.000Z",
    updatedAt: "2026-07-15T12:00:00.000Z",
  }
}

function retryableClaudeProfile() {
  return {
    ...readyClaudeProfile(),
    status: "error" as const,
    verification: "expired" as const,
    enrollment: {
      ...credentialMutation().profile.enrollment!,
      status: "expired" as const,
    },
  }
}

function binding(overrides: Partial<ClaudeCredentialEnrollmentBinding> = {}): ClaudeCredentialEnrollmentBinding {
  return {
    accountId: "account-1",
    enrollmentId: "enrollment-1",
    profileId: "profile-claude",
    targetVersion: 1,
    enrollmentExpiresAt: "2099-07-15T12:30:00.000Z",
    realmId: "realm-1",
    kernelTarget: "kernel-1",
    sessionId: "session-1",
    attachmentId: "attachment-1",
    agentId: "agent-1",
    ...overrides,
  }
}

function kernelArmResponse(
  value: ClaudeCredentialEnrollmentBinding,
  expiresAtMs = Date.now() + 60_000,
): Record<string, unknown> {
  return {
    DeploymentCredentialEnrollmentArmed: {
      enrollment_id: value.enrollmentId,
      profile_id: value.profileId,
      target_version: value.targetVersion,
      session_id: value.sessionId,
      agent_id: value.agentId,
      expires_at_ms: expiresAtMs,
    },
  }
}

function callbackArmResult(
  value: ClaudeCredentialEnrollmentBinding,
): DeploymentCredentialCallbackChannelResult {
  return {
    channel: {
      status: "armed",
      accountId: value.accountId,
      enrollmentId: value.enrollmentId,
      profileId: value.profileId,
      targetVersion: value.targetVersion,
      realmId: value.realmId,
      kernelTarget: value.kernelTarget,
      sessionId: value.sessionId,
      agentId: value.agentId,
      armedAt: "2026-07-15T12:01:00.000Z",
      expiresAt: value.enrollmentExpiresAt,
    },
  }
}

const cloudProfile: RelayCloudProfile = {
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

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  })
}
