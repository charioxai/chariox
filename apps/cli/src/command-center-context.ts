import type { BackendProviderId, ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"
import type { CommandNode } from "./command-center-tree-projection.js"

export type CommandCenterWorkflowRegistryEntry = {
  name: string
  sourceScope: "workspace" | "user" | "builtin"
  sourceKind: "single_file" | "source_directory"
  endpoints?: readonly string[]
  queues?: readonly string[]
}

export type CommandCenterContext = {
  commandTree: readonly CommandNode[]
  providerCatalog: ProviderCatalog
  providerCommandCatalogs: ProviderCommandCatalogs
  currentProvider: BackendProviderId
  focusedProvider: BackendProviderId | null
  currentModel: string
  currentVariant: string
  workflowRegistryEntries?: readonly CommandCenterWorkflowRegistryEntry[]
}

export type CommandCenterDynamicContext = Pick<
  CommandCenterContext,
  "providerCatalog" | "currentProvider" | "currentModel" | "currentVariant"
>

export type CommandCenterProviderNamespaceContext = Pick<
  CommandCenterContext,
  "commandTree" | "focusedProvider" | "providerCommandCatalogs"
>
