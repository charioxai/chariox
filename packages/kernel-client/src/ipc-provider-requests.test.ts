import assert from "node:assert/strict"
import test from "node:test"

import { setProviderAccountCredentialRequest, startProviderLoginRequest } from "./ipc-provider-requests.js"
import * as providerRequests from "./ipc-provider-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("provider login request carries the selected enrollment method", () => {

  const request = startProviderLoginRequest("codex", "work", "device_code") as {
    StartProviderLogin: Record<string, unknown>
  }
  assert.equal(request.StartProviderLogin.provider, "codex")
  assert.equal(request.StartProviderLogin.account_profile, "work")
  assert.equal(request.StartProviderLogin.method, "device_code")
})

test("provider login request omits the method key for default enrollment", () => {
  const request = startProviderLoginRequest("claude") as {
    StartProviderLogin: Record<string, unknown>
  }
  assert.equal("method" in request.StartProviderLogin, false)
})

test("native account import requests the kernel scope without accepting a client path", () => {
  assert.equal(typeof providerRequests.importNativeProviderAccountProfileRequest, "function")
  assert.deepEqual(providerRequests.importNativeProviderAccountProfileRequest("claude"), {
    ImportNativeProviderAccountProfile: { provider: "claude" },
  })
})

test("provider setup token request carries account scope and explicit replacement", () => {
  assert.deepEqual(
    setProviderAccountCredentialRequest(
      "claude",
      "work",
      "secret-token",
      true,
      { sessionId: "session-1", agentId: "agent-1" },
    ),
    {
      SetProviderAccountCredential: {
        session_id: "session-1",
        agent_id: "agent-1",
        provider: "claude",
        account_profile: "work",
        value: "secret-token",
        overwrite: true,
      },
    },
  )
})

test("MP-08/MP-11 setup-token run carries only scope and replacement policy", () => {
  assert.deepEqual(setProviderAccountCredentialRequest("claude", "work", "", true, { sessionId: "session-1" }, true), {
    SetProviderAccountCredential: { provider: "claude", account_profile: "work", value: "", run: true, overwrite: true, session_id: "session-1" },
  })
})


test("MP-08 / MP-10 / MP-11 protocol 518 catalogs use the ordinary provider scope for revision checks", async () => {
  const {getClientCatalogRevisionsRequest,clientCatalogRevisionsMinimumProtocolVersion,getProviderCatalogRequest} = await import("./ipc-provider-requests.js")
  const scope = {provider:"codex",accountProfile:"work",executionLocation:{kind:"worker" as const,kernel_ref:"worker-one"}}
  assert.equal(clientCatalogRevisionsMinimumProtocolVersion,518)
  assert.deepEqual(getClientCatalogRevisionsRequest(scope),{GetClientCatalogRevisions:getProviderCatalogRequest(scope).GetProviderCatalog})
})

test("MP-08 / MP-10 conditional catalogs carry known revisions and the ordinary scope", async () => {
  const {getClientCatalogSnapshotRequest,clientCatalogSnapshotMinimumProtocolVersion,getProviderCatalogRequest} = await import("./ipc-provider-requests.js")
  const scope = {provider: "codex", accountProfile: "work"}
  const known = {provider_accounts: "a".repeat(64), provider_catalog: "b".repeat(64), provider_commands: "c".repeat(64), terminal_commands: `sha256:${"d".repeat(64)}`}
  assert.equal(clientCatalogSnapshotMinimumProtocolVersion, 534)
  assert.deepEqual(getClientCatalogSnapshotRequest(scope, known), {GetClientCatalogSnapshot: {scope: getProviderCatalogRequest(scope).GetProviderCatalog, known}})
  assert.equal(getClientCatalogSnapshotRequest().GetClientCatalogSnapshot.known, null)
})

test("recipient account copy carries metadata and an exact operation only", async () => {
  const { listProviderAccountCopyOffersRequest, copyProviderAccountFromKernelRequest } = await import("./provider-account-copy.js")
  assert.deepEqual(listProviderAccountCopyOffersRequest(), { ListProviderAccountCopyOffers: {} })
  assert.deepEqual(copyProviderAccountFromKernelRequest({ source_kernel_id: "a", source_machine_id: "ma", source_label: "A", source_key_thumbprint: "pin", target_kernel_id: "b", provider: "claude", account_profile: "work", label: "Work" }, "op"), {
    CopyProviderAccountFromKernel: { provider: "claude", account_profile: "work", source_kernel_id: "a", source_key_thumbprint: "pin", operation_id: "op", replace: false },
  })
  // A locked or never-created Vault is opened in the session the person copies from.
  assert.deepEqual(copyProviderAccountFromKernelRequest({ source_kernel_id: "a", source_machine_id: "ma", source_label: "A", source_key_thumbprint: "pin", target_kernel_id: "b", provider: "claude", account_profile: "work", label: "Work" }, "op", { sessionId: "s1", agentId: "agent-1" }, 711), {
    CopyProviderAccountFromKernel: { session_id: "s1", agent_id: "agent-1", provider: "claude", account_profile: "work", source_kernel_id: "a", source_key_thumbprint: "pin", operation_id: "op", replace: false },
  })
  // Kernels before 711 reject those fields, so they get the 710 request.
  for (const protocol of [710, undefined]) {
    assert.deepEqual(copyProviderAccountFromKernelRequest({ source_kernel_id: "a", source_machine_id: "ma", source_label: "A", source_key_thumbprint: "pin", target_kernel_id: "b", provider: "claude", account_profile: "work", label: "Work" }, "op", { sessionId: "s1", agentId: "agent-1" }, protocol), {
      CopyProviderAccountFromKernel: { provider: "claude", account_profile: "work", source_kernel_id: "a", source_key_thumbprint: "pin", operation_id: "op", replace: false },
    })
  }
})
