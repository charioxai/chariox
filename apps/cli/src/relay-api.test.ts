import assert from "node:assert/strict"
import test from "node:test"

import { LOCAL_DAEMON_PROTOCOL_VERSION } from "@chariox/kernel-client/kernel-types"

import { issueKernelCloudRelayClientToken, joinKernelTerminalPairingLink, logoutCloudRelay, resolveKernelClientConnection } from "./relay-api.js"
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

function connectionResponse(token: string, expiresAt: string | null | undefined) {
  return { KernelClientConnectionResolved: { connection: {
    relay_url: "wss://relay.example", relay_token: token,
    target_daemon_id: "kernel-2", target_daemon_alias: "other-kernel",
    machine_id: "machine-2", kernel_id: "kernel-2",
    ...(expiresAt !== undefined ? { token_expires_at: expiresAt } : {}),
  } } }
}

const connectionInput = { kernelRef: "kernel-2", machineRef: "machine-2", clientId: "cli-1", publicKeyThumbprint: "bootstrap-thumbprint" }

test("MP-08/MP-11 waiting-room resolution preserves explicit self-hosted shared tokens", async () => {
  for (const expiresAt of [undefined, null]) {
    const requests: unknown[] = []
    const connection = await resolveKernelClientConnection(fakeClient(async request => {
      requests.push(request)
      if ("CloudRelayStatus" in (request as object)) return { CloudRelayStatus: { profile: null } }
      return connectionResponse("shared-token", expiresAt)
    }), connectionInput)
    assert.equal(connection.relayToken, "shared-token")
    assert.equal(connection.tokenExpiresAtMs, null)
    assert.equal(connection.targetDaemonId, "kernel-2")
    assert.equal(connection.machineId, "machine-2")
    assert.equal((requests[0] as any).ResolveKernelClientConnection.public_key_thumbprint, "bootstrap-thumbprint")
  }
})

test("MP-08/MP-11 waiting-room resolution rejects unbound hosted and ambiguous tokens", async () => {
  for (const expiresAt of ["2099-01-01T00:00:00Z", undefined, null]) {
    for (const profile of [{}, undefined]) {
      await assert.rejects(resolveKernelClientConnection(fakeClient(async request => "CloudRelayStatus" in (request as object)
        ? { CloudRelayStatus: { profile } } : connectionResponse("shared-token", expiresAt)), connectionInput), /requires a relay token bound/)
    }
  }
  for (const expiresAt of ["2099-01-01T00:00:00Z", null]) {
    await assert.rejects(resolveKernelClientConnection(fakeClient(async () => connectionResponse(relayToken("foreign-thumbprint"), expiresAt)), connectionInput), /did not bind the token/)
  }
})

test("MP-08/MP-11 waiting-room resolution accepts the bound Cloud token", async () => {
  const connection = await resolveKernelClientConnection(fakeClient(async () => connectionResponse(relayToken("bootstrap-thumbprint"), "2099-01-01T00:00:00Z")), connectionInput)
  assert.equal(connection.relayToken, relayToken("bootstrap-thumbprint"))
  assert.equal(connection.tokenExpiresAtMs, Date.parse("2099-01-01T00:00:00Z"))
})

test("CLI token and terminal join requests bind the actual bootstrap thumbprint", async () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 473)
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

test("MP-08 Cloud logout accepts the kernel's unit-variant acknowledgement", async () => {
  const requests: unknown[] = []
  const client = {
    send: async (request: unknown) => {
      requests.push(request)
      return "CloudRelayLoggedOut"
    },
  } as unknown as LocalIpcClient

  await logoutCloudRelay(client)

  assert.equal(requests.length, 1)
  const unexpected = { send: async () => "SomethingElse" } as unknown as LocalIpcClient
  await assert.rejects(logoutCloudRelay(unexpected), /expected CloudRelayLoggedOut/)
})
