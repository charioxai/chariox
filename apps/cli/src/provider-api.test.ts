import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import { copyProviderAccountFromKernel, getProviderCatalog, getProviderCommandCatalogs, runProviderSetupToken } from "./provider-api.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"

test("getProviderCatalog marks daemon catalog source", async () => {
  const catalog = await getProviderCatalog(clientReturning({
    ProviderCatalog: {
      catalog: {
        all: [],
        default: {},
        connected: [],
      } satisfies ProviderCatalog,
    },
  }))

  assert.equal(catalog.source, "daemon")
})

test("getProviderCatalog marks local fallback when daemon lookup fails", async () => {
  const warnings: Array<Record<string, unknown>> = []
  const catalog = await getProviderCatalog(clientRejecting(new Error("catalog down")), {
    warn: (_message: string, fields?: Record<string, unknown>) => {
      warnings.push(fields ?? {})
    },
  } as never)

  assert.equal(catalog.source, "local_fallback")
  assert.match(catalog.unavailable_reason ?? "", /catalog down/)
  assert.match(String(warnings[0]?.error ?? ""), /catalog down/)
})

test("getProviderCommandCatalogs marks daemon catalog source", async () => {
  const catalogs = await getProviderCommandCatalogs(clientReturning({
    ProviderCommandCatalogs: {
      catalogs: {
        opencode: {
          provider: "opencode",
          source: "shipped",
          discovery: "none",
          commands: [],
        },
        codex: {
          provider: "codex",
          source: "shipped",
          discovery: "none",
          commands: [],
        },
        "claude-headless": {
          provider: "claude-headless",
          source: "shipped",
          discovery: "none",
          commands: [],
        },
        "claude-p": {
          provider: "claude-p",
          source: "shipped",
          discovery: "none",
          commands: [],
        },
      } satisfies ProviderCommandCatalogs,
    },
  }))

  assert.equal(catalogs.codex.catalog_source, "daemon")
})

test("getProviderCommandCatalogs marks local fallback when daemon lookup fails", async () => {
  const warnings: Array<Record<string, unknown>> = []
  const catalogs = await getProviderCommandCatalogs(clientRejecting(new Error("command catalog down")), {
    warn: (_message: string, fields?: Record<string, unknown>) => {
      warnings.push(fields ?? {})
    },
  } as never)

  assert.equal(catalogs.codex.catalog_source, "local_fallback")
  assert.match(catalogs.codex.unavailable_reason ?? "", /command catalog down/)
  assert.match(String(warnings[0]?.error ?? ""), /command catalog down/)
})

function clientReturning(response: Record<string, unknown>): LocalIpcClient {
  return {
    send: async () => response,
  } as unknown as LocalIpcClient
}

function clientRejecting(error: Error): LocalIpcClient {
  return {
    send: async () => {
      throw error
    },
  } as unknown as LocalIpcClient
}

// MP-08/MP-11: a main410 daemon rejects G2's run field with an upgrade diagnostic.
test("setup-token run names protocol411 when the daemon lacks its run field", async () => {
  await assert.rejects(
    runProviderSetupToken(clientRejecting(new Error("unknown field `run`")), "work", false),
    /Claude setup token capture requires kernel protocol 411 or newer/,
  )
})

const copyOffer = { source_kernel_id: "a", source_machine_id: "ma", source_label: "A", source_key_thumbprint: "pin", target_kernel_id: "b", provider: "claude", account_profile: "work", label: "Work" }
async function sentCopyRequest(kernelProtocol: number | undefined): Promise<Record<string, any>> {
  const sent: Array<Record<string, any>> = []
  const client = {
    kernelProtocolVersion: async () => kernelProtocol,
    send: async (request: Record<string, any>) => {
      sent.push(request)
      return request.RelayStatus === null
        ? { RelayStatus: { status: { daemon_id: "b" } } }
        : { ProviderAccountCopied: { receipt: { operation_id: "op", provider: "claude", source_profile_id: "work", target_kernel_id: "b", account_profile: "copied" } } }
    },
  } as unknown as LocalIpcClient
  await copyProviderAccountFromKernel(client, copyOffer, "op", { sessionId: "s1", agentId: "agent-1" })
  return sent.find(item => item.CopyProviderAccountFromKernel)!.CopyProviderAccountFromKernel
}

test("a login copy names the session that shows the Vault prompt", async () => {
  const request = await sentCopyRequest(711)
  assert.equal(request.session_id, "s1")
  assert.equal(request.agent_id, "agent-1")
})

// A 710 kernel rejects the unknown session_id/agent_id before consent; so does a kernel of unknown version.
for (const [name, kernelProtocol] of [["710", 710], ["unknown", undefined]] as const) {
  test(`a login copy sends the 710 request to a ${name} kernel`, async () => {
    const request = await sentCopyRequest(kernelProtocol)
    assert.deepEqual(Object.keys(request).sort(), ["account_profile", "operation_id", "provider", "replace", "source_kernel_id", "source_key_thumbprint"])
  })
}
