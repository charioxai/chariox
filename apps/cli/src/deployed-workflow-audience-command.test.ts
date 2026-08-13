import assert from "node:assert/strict"
import test from "node:test"

import {
  executeDeployedWorkflowCommand,
} from "./deployed-workflow-command.js"
import {
  formatDeploymentAudience,
} from "./deployed-workflow-audience-command.js"
import type { DeploymentAudiencePolicySummary } from "./deployed-workflow-types.js"
import type { RelayCloudProfile } from "./preferences.js"

test("deployed workflow TUI command drives the complete audience lifecycle", async () => {
  const originalFetch = globalThis.fetch
  const calls: Array<{
    readonly method: string
    readonly pathname: string
    readonly search: string
    readonly body: Record<string, unknown> | null
  }> = []
  globalThis.fetch = async (input, init) => {
    const url = new URL(String(input))
    const body = typeof init?.body === "string" ? JSON.parse(init.body) as Record<string, unknown> : null
    calls.push({ method: init?.method ?? "GET", pathname: url.pathname, search: url.search, body })
    if (url.pathname.endsWith("/grants") && body?.status === "invited") {
      return jsonResponse({ audience, grantToken: "chariox_audience_invite_one_time_secret" }, 201)
    }
    if (url.pathname.endsWith("/api-keys")) {
      return jsonResponse({ audience, apiKey: "chariox_app_one_time_secret" }, 201)
    }
    if (url.pathname.endsWith("/webhook-keys")) {
      return jsonResponse({ audience, webhookSecret: "chariox_webhook_one_time_secret" }, 201)
    }
    return jsonResponse({ audience })
  }

  try {
    const shown = await executeDeployedWorkflowCommand(profile, [
      "audience", "show", "project/one", "environment/one",
    ])
    const policy = await executeDeployedWorkflowCommand(profile, [
      "audience", "policy", "project/one", "environment/one", "public",
      "--roles", "invoke,admin,invoke",
    ])
    const invitation = await executeDeployedWorkflowCommand(profile, [
      "audience", "grant", "add", "project/one", "environment/one",
      "email-domain", "customer.test", "--roles", "invoke", "--status", "invited",
      "--expires-seconds", "604800",
    ])
    await executeDeployedWorkflowCommand(profile, [
      "audience", "grant", "revoke", "project/one", "environment/one", "grant/one",
    ])
    const key = await executeDeployedWorkflowCommand(profile, [
      "audience", "key", "create", "project/one", "environment/one", "Production MCP",
      "--roles", "invoke", "--expires-seconds", "7776000",
    ])
    await executeDeployedWorkflowCommand(profile, [
      "audience", "key", "revoke", "project/one", "environment/one", "key/one",
    ])
    const jwt = await executeDeployedWorkflowCommand(profile, [
      "audience", "jwt", "create", "project/one", "environment/one", "Customer app",
      "--issuer", "https://identity.customer.test/", "--audience", "support-production",
      "--jwks-json", '{"keys":[{"kty":"OKP","kid":"customer-key-1","alg":"EdDSA","crv":"Ed25519","x":"public"}]}',
      "--roles", "invoke", "--roles-claim", "permissions", "--expires-seconds", "7776000",
    ])
    await executeDeployedWorkflowCommand(profile, [
      "audience", "jwt", "revoke", "project/one", "environment/one", "jwt/one",
    ])
    const webhook = await executeDeployedWorkflowCommand(profile, [
      "audience", "webhook", "create", "project/one", "environment/one", "CRM",
      "--roles", "invoke", "--replay-seconds", "300", "--expires-seconds", "7776000",
    ])
    await executeDeployedWorkflowCommand(profile, [
      "audience", "webhook", "revoke", "project/one", "environment/one", "webhook/one",
    ])
    const accepted = await executeDeployedWorkflowCommand(profile, [
      "audience", "invite", "accept", "chariox_audience_invite_one_time_secret",
    ])
    const restricted = await executeDeployedWorkflowCommand(profile, [
      "audience", "policy", "project/one", "environment/one", "restricted",
    ])

    const base = "/deployment-projects/project%2Fone/environments/environment%2Fone/audience"
    assert.deepEqual(calls, [
      { method: "GET", pathname: base, search: "?accountId=account-1", body: null },
      {
        method: "POST",
        pathname: `${base}/policy`,
        search: "",
        body: { accountId: "account-1", mode: "public", defaultRoles: ["admin", "invoke"] },
      },
      {
        method: "POST",
        pathname: `${base}/grants`,
        search: "",
        body: {
          accountId: "account-1",
          kind: "email_domain",
          subject: "customer.test",
          roles: ["invoke"],
          status: "invited",
          expiresInSeconds: 604800,
        },
      },
      {
        method: "POST",
        pathname: `${base}/grants/grant%2Fone/revoke`,
        search: "",
        body: { accountId: "account-1" },
      },
      {
        method: "POST",
        pathname: `${base}/api-keys`,
        search: "",
        body: {
          accountId: "account-1",
          name: "Production MCP",
          roles: ["invoke"],
          expiresInSeconds: 7776000,
        },
      },
      {
        method: "POST",
        pathname: `${base}/api-keys/key%2Fone/revoke`,
        search: "",
        body: { accountId: "account-1" },
      },
      {
        method: "POST",
        pathname: `${base}/jwt-issuers`,
        search: "",
        body: {
          accountId: "account-1",
          name: "Customer app",
          issuer: "https://identity.customer.test/",
          audience: "support-production",
          jwks: [{ kty: "OKP", kid: "customer-key-1", alg: "EdDSA", crv: "Ed25519", x: "public" }],
          roles: ["invoke"],
          rolesClaim: "permissions",
          expiresInSeconds: 7776000,
        },
      },
      {
        method: "POST",
        pathname: `${base}/machine-credentials/jwt%2Fone/revoke`,
        search: "",
        body: { accountId: "account-1" },
      },
      {
        method: "POST",
        pathname: `${base}/webhook-keys`,
        search: "",
        body: {
          accountId: "account-1",
          name: "CRM",
          roles: ["invoke"],
          replayWindowSeconds: 300,
          expiresInSeconds: 7776000,
        },
      },
      {
        method: "POST",
        pathname: `${base}/machine-credentials/webhook%2Fone/revoke`,
        search: "",
        body: { accountId: "account-1" },
      },
      {
        method: "POST",
        pathname: "/deployment-audience-invitations/accept",
        search: "",
        body: { grantToken: "chariox_audience_invite_one_time_secret" },
      },
      {
        method: "POST",
        pathname: `${base}/policy`,
        search: "",
        body: { accountId: "account-1", mode: "restricted", defaultRoles: [] },
      },
    ])
    assert.match(shown.notice, /route route-invoke http/)
    assert.match(shown.notice, /grant grant-1 active/)
    assert.match(shown.notice, /api_key key-1 active/)
    assert.match(policy.footer, /audience public/)
    assert.match(invitation.notice, /invitation_token chariox_audience_invite_one_time_secret/)
    assert.equal(invitation.footer, "deployment audience invitation created; token shown once")
    assert.match(key.notice, /api_key_secret chariox_app_one_time_secret/)
    assert.equal(key.footer, "deployment audience API key created; secret shown once")
    assert.equal(jwt.footer, "deployment audience JWT issuer created")
    assert.match(webhook.notice, /webhook_secret chariox_webhook_one_time_secret/)
    assert.equal(webhook.footer, "deployment audience webhook key created; secret shown once")
    assert.doesNotMatch(accepted.notice, /chariox_audience_invite_one_time_secret/)
    assert.equal(restricted.footer, "deployment audience restricted")
  } finally {
    globalThis.fetch = originalFetch
  }
})

test("deployed workflow audience command rejects unsafe or ambiguous credentials", async () => {
  await assert.rejects(
    executeDeployedWorkflowCommand(profile, [
      "audience", "policy", "project-1", "environment-1", "public",
    ]),
    /requires --roles/,
  )
  await assert.rejects(
    executeDeployedWorkflowCommand(profile, [
      "audience", "grant", "add", "project-1", "environment-1",
      "email", "user@example.test", "--roles", "invoke", "--no-expiry",
    ]),
    /invitations must expire/,
  )
  await assert.rejects(
    executeDeployedWorkflowCommand(profile, [
      "audience", "key", "create", "project-1", "environment-1", "bad", "--roles", "invalid role",
    ]),
    /comma-separated deployment roles/,
  )
  await assert.rejects(
    executeDeployedWorkflowCommand(profile, [
      "audience", "grant", "add", "project-1", "environment-1",
      "ip", "127.0.0.1", "--roles", "invoke",
    ]),
    /kind must be email, email-domain, or account/,
  )
  await assert.rejects(
    executeDeployedWorkflowCommand(profile, [
      "audience", "jwt", "create", "project-1", "environment-1", "bad",
      "--issuer", "https://issuer.test", "--audience", "app", "--jwks-json", "not-json", "--roles", "invoke",
    ]),
    /valid JSON/,
  )
  await assert.rejects(
    executeDeployedWorkflowCommand(profile, [
      "audience", "webhook", "create", "project-1", "environment-1", "bad",
      "--roles", "invoke", "--replay-seconds", "5",
    ]),
    /between 30 and 900/,
  )
})

test("deployment audience formatting exposes prefixes and policy state but no credential material", () => {
  const formatted = formatDeploymentAudience(audience)
  assert.match(formatted, /prefix chariox_app_prefix/)
  assert.match(formatted, /default_roles invoke/)
  assert.match(formatted, /jwt_issuer jwt-1 active/)
  assert.match(formatted, /webhook_key webhook-1 active/)
  assert.doesNotMatch(formatted, /one_time_secret|session-token/)
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

const audience: DeploymentAudiencePolicySummary = {
  id: "audience-environment-1",
  accountId: "account-1",
  projectId: "project-1",
  environmentId: "environment-1",
  mode: "public",
  defaultRoles: ["invoke"],
  routes: [{ id: "route-invoke", path: "/invoke", transport: "http", requiredRoles: ["invoke"] }],
  grants: [{
    id: "grant-1",
    policyId: "audience-environment-1",
    kind: "email",
    subject: "user@example.test",
    roles: ["invoke"],
    status: "active",
    expiresAt: "2030-01-01T00:00:00.000Z",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }],
  apiKeys: [{
    id: "key-1",
    policyId: "audience-environment-1",
    name: "Production MCP",
    keyPrefix: "chariox_app_prefix",
    roles: ["invoke"],
    expiresAt: "2030-01-01T00:00:00.000Z",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }],
  jwtIssuers: [{
    id: "jwt-1",
    policyId: "audience-environment-1",
    name: "Customer app",
    keyId: "jwt_customer",
    issuer: "https://identity.customer.test/",
    audience: "support-production",
    jwkKeyIds: ["customer-key-1"],
    roles: ["invoke"],
    rolesClaim: "permissions",
    expiresAt: "2030-01-01T00:00:00.000Z",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }],
  webhookKeys: [{
    id: "webhook-1",
    policyId: "audience-environment-1",
    name: "CRM",
    keyId: "whk_customer",
    roles: ["invoke"],
    replayWindowSeconds: 300,
    expiresAt: "2030-01-01T00:00:00.000Z",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  }],
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  })
}
