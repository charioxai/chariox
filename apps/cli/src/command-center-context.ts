import type { BackendProviderId, ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"

export type CommandCenterContext = {
  providerCatalog: ProviderCatalog
  providerCommandCatalogs: ProviderCommandCatalogs
  currentProvider: BackendProviderId
  focusedProvider: BackendProviderId | null
  currentModel: string
  currentVariant: string
}

export type CommandCenterDynamicContext = Pick<
  CommandCenterContext,
  "providerCatalog" | "currentProvider" | "currentModel" | "currentVariant"
>

export type CommandCenterProviderNamespaceContext = Pick<
  CommandCenterContext,
  "focusedProvider" | "providerCommandCatalogs"
>
