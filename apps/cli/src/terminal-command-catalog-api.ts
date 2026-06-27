import type {
  TerminalCommandCatalog,
} from "@arroba/kernel-client/kernel-types"
import {
  getTerminalCommandCatalogRequest,
} from "./ipc-requests.js"
import type { LocalIpcClient } from "./ipc.js"
import type { ArrobaLogger } from "./logging.js"
import { expectVariant } from "./ipc-response.js"

export async function getTerminalCommandCatalog(
  client: LocalIpcClient,
  logger?: ArrobaLogger | null,
): Promise<TerminalCommandCatalog> {
  const response = await client.send<Record<string, unknown>>(getTerminalCommandCatalogRequest())
  const payload = expectVariant<{ catalog: TerminalCommandCatalog }>(response, "TerminalCommandCatalog")
  logger?.info("Received terminal command catalog from daemon", {
    revision: payload.catalog.revision,
    root_count: payload.catalog.nodes.length,
  })
  return payload.catalog
}
