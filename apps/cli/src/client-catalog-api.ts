// MP-08 / MP-10 / MP-11: small scoped inventory revisions; older kernels retain full reads.
import { getClientCatalogRevisionsRequest, clientCatalogRevisionsMinimumProtocolVersion, type ClientCatalogRevisions } from "./ipc-requests.js"
import type { LocalIpcClient } from "./ipc.js"
import { expectVariant } from "./ipc-response.js"
import { KernelProtocolMinimumError, sendWithProtocolMinimum } from "./protocol-minimum-diagnostic.js"
export async function getClientCatalogRevisions(client: LocalIpcClient, options: Parameters<typeof getClientCatalogRevisionsRequest>[0] = {}): Promise<ClientCatalogRevisions | null> {
  try {
    const response = await sendWithProtocolMinimum<Record<string, unknown>>(client.send.bind(client), getClientCatalogRevisionsRequest(options), {
      capability: "catalog revisions", requestVariant: "GetClientCatalogRevisions", minimumProtocolVersion: clientCatalogRevisionsMinimumProtocolVersion,
    })
    const { revisions } = expectVariant<{ revisions: ClientCatalogRevisions }>(response, "ClientCatalogRevisions")
    if (!revisions || !/^sha256:[a-f0-9]{64}$/.test(revisions.terminal_commands) || ![revisions?.provider_accounts, revisions?.provider_catalog, revisions?.provider_commands].every(value => typeof value === "string" && /^[a-f0-9]{64}$/.test(value))) throw new Error("malformed client catalog revisions")
    return revisions
  } catch (error) { if (error instanceof KernelProtocolMinimumError) return null; throw error }
}
