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

test("account copy returns the receiving Vault wait without claiming installation", async () => {
  const offer = { source_kernel_id: "source", source_machine_id: "source-machine", source_label: "Source", source_key_thumbprint: "pin", target_kernel_id: "target", provider: "claude", account_profile: "work", label: "Work" }
  const wait = { operation_id: "operation", source_kernel_id: "source", target_kernel_id: "target", target_label: "Target", expires_at_ms: Date.now() + 600_000 }
  const client = { send: async (request: Record<string, unknown>) => "RelayStatus" in request ? { RelayStatus: { status: { daemon_id: "target" } } } : { ProviderAccountCopyWaiting: { wait } } } as unknown as LocalIpcClient
  let result: unknown
  await assert.doesNotReject(async () => { result = await copyProviderAccountFromKernel(client, offer, "operation") }, "a waiting copy is a valid result before installation")
  assert.deepEqual(result, wait)
})

test("account copy rejects waiting metadata for a different operation", async () => {
  const offer = { source_kernel_id: "source", source_machine_id: "source-machine", source_label: "Source", source_key_thumbprint: "pin", target_kernel_id: "target", provider: "claude", account_profile: "work", label: "Work" }
  const client = { send: async (request: Record<string, unknown>) => "RelayStatus" in request ? { RelayStatus: { status: { daemon_id: "target" } } } : { ProviderAccountCopyWaiting: { wait: { operation_id: "other", source_kernel_id: "source", target_kernel_id: "target", target_label: "Target", expires_at_ms: Date.now() + 600_000 } } } } as unknown as LocalIpcClient
  await assert.rejects(copyProviderAccountFromKernel(client, offer, "operation"))
})

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

test("waiting account copies require both bound kernels and a finite expiry", async () => {
  const offer = { source_kernel_id: "source", source_machine_id: "machine", source_label: "Source", source_key_thumbprint: "pin", target_kernel_id: "target", provider: "claude", account_profile: "work", label: "Work" }
  const wait = { operation_id: "operation", source_kernel_id: "source", target_kernel_id: "target", target_label: "Target", expires_at_ms: Date.now() + 600_000 }
  for (const invalid of [{ ...wait, source_kernel_id: "other" }, { ...wait, target_kernel_id: "other" }, { ...wait, expires_at_ms: Number.NaN }]) {
    const client = { send: async (request: Record<string, unknown>) => "RelayStatus" in request ? { RelayStatus: { status: { daemon_id: "target" } } } : { ProviderAccountCopyWaiting: { wait: invalid } } } as unknown as LocalIpcClient
    await assert.rejects(copyProviderAccountFromKernel(client, offer, "operation"))
  }
})
