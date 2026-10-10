import assert from "node:assert/strict"
import test from "node:test"

import { LOCAL_DAEMON_PROTOCOL_VERSION } from "@chariox/kernel-client/kernel-types"

import { issueKernelCloudRelayClientToken, joinKernelTerminalPairingLink } from "./relay-api.js"
import type { LocalIpcClient } from "./ipc.js"

function fakeClient(send: (request: unknown) => Promise<unknown>): LocalIpcClient {
  return { send } as unknown as LocalIpcClient
}

function relayToken(thumbprint: string, allowedTargets?: unknown) {
  const payload = Buffer.from(JSON.stringify({ public_key_thumbprint: thumbprint, ...(allowedTargets !== undefined ? { allowed_targets: allowedTargets } : {}) })).toString("base64url")
  return `eyJhbGciOiJub25lIn0.${payload}.signature`
}

function tokenResponse(thumbprint = "bootstrap-thumbprint", allowedTargets?: unknown) {
  return {
    CloudRelayClientTokenIssued: {
      profile: {
        api_url: "https://cloud.example",
        email: "cli@example.test",
        account_id: "account-1",
        user_id: "user-1",
        account_slug: "account",
        realm_id: "realm-1",
        relay_url: "wss://relay.example",
        issuer_id: "issuer-1",
      },
      token: {
        relay_url: "wss://relay.example",
        relay_token: relayToken(thumbprint, allowedTargets),
        token_expires_at: "2099-01-01T00:00:00Z",
      },
    },
  }
}

test("CLI token and terminal join requests bind the actual bootstrap thumbprint", async () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 478)
  const requests: unknown[] = []
  const client = fakeClient(async (request) => {
    requests.push(request)
    if ("IssueCloudRelayClientToken" in (request as object)) return tokenResponse()
    return {
      TerminalPairingLinkJoined: {
        terminal: { terminal_id: "terminal-1", terminal_type: "cli", paired_at_ms: 1, revoked: false },
        pairing: {
          intent: "client",
          subject_id: "terminal-1",
          relay_url: "wss://relay.example",
          target_daemon_id: "home-1",
          public_key_thumbprint: "bootstrap-thumbprint",
          paired_at_ms: 1,
        },
        relay_token: relayToken("bootstrap-thumbprint"),
      },
    }
  })

  await issueKernelCloudRelayClientToken(
    client,
    "home",
    "terminal-1",
    null,
    "bootstrap-thumbprint",
  )
  const joined = await joinKernelTerminalPairingLink(
    client,
    "chariox-terminal-pair-v1.fixture",
    "terminal-1",
    "bootstrap-thumbprint",
  )

  assert.equal(joined.relay_token, relayToken("bootstrap-thumbprint"))
  assert.equal(requests[0] && (requests[0] as any).IssueCloudRelayClientToken.public_key_thumbprint, "bootstrap-thumbprint")
  assert.equal(requests[1] && (requests[1] as any).JoinTerminalPairingLink.public_key_thumbprint, "bootstrap-thumbprint")
})

test("key-bound token issuance reports the protocol 349 minimum", async () => {
  const client = fakeClient(async () => {
    throw new Error("unknown field `public_key_thumbprint`")
  })
  await assert.rejects(
    issueKernelCloudRelayClientToken(client, "home", "cli-1", null, "thumbprint"),
    /CLI key-bound relay tokens requires kernel protocol 349 or newer/,
  )
})

test("key-bound issuance rejects an unbound relay token instead of returning it", async () => {
  const client = fakeClient(async () => tokenResponse("foreign-thumbprint"))
  await assert.rejects(
    issueKernelCloudRelayClientToken(client, "home", "cli-1", null, "bootstrap-thumbprint"),
    /did not bind the token to this CLI's public key/,
  )
})


test("client token projects a singleton canonical target without changing its scope", async () => {
  const response = tokenResponse("bootstrap-thumbprint", ["kernel-canonical-1"])
  const issued = await issueKernelCloudRelayClientToken(
    fakeClient(async () => response), "home-alias", "cli-1", null, "bootstrap-thumbprint",
  )
  assert.equal(issued.targetDaemonId, "kernel-canonical-1")
  assert.equal(issued.relayToken, response.CloudRelayClientTokenIssued.token.relay_token)
})

test("alias-scoped and ambiguous client tokens preserve the requested alias", async () => {
  for (const targets of [undefined, null, [], ["home-alias"], ["kernel-1", "kernel-2"], [""], ["   "], [7], "kernel-1"]) {
    const issued = await issueKernelCloudRelayClientToken(
      fakeClient(async () => tokenResponse("bootstrap-thumbprint", targets)),
      "home-alias", "cli-1", null, "bootstrap-thumbprint",
    )
    assert.equal(issued.targetDaemonId, undefined)
  }
})

test("a second terminal obtains public enrollment status directly from its attached kernel", async () => {
  const { getKernelCloudRelayProfile, connectKernelCloudRelay } = await import("./relay-api.js")
  const privateResponse = tokenResponse().CloudRelayClientTokenIssued.profile
  const profile = {...privateResponse, kernel_id: "kernel-a", kernel_enrolled: true, machine_credential: "synthetic-machine", cloud_session_token: "synthetic-session", kernel_credential: "synthetic-kernel"}
  const client = fakeClient(async request => "CloudRelayStatus" in (request as object) ? {CloudRelayStatus: {profile}} : {CloudRelayConnected: {profile, status: {configured: true, connected: true}}})
  const publicProfile = await getKernelCloudRelayProfile(client)
  assert.equal(publicProfile?.kernelId, "kernel-a")
  assert.ok(!JSON.stringify(publicProfile).includes("synthetic"))
  const connected = await connectKernelCloudRelay(client)
  assert.equal(connected.profile.kernelEnrolled, true)
  assert.ok(!("relayToken" in connected))
})
