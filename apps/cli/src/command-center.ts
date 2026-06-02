import {
  providerNamespace,
  providerSupportsNamespaceCommands,
} from "./provider-command-catalog.js"
import {
  buildModelItems,
  buildProviderItems,
  buildProviderNamespaceItems,
  buildVariantItems,
  buildViewItems,
} from "./command-center-dynamic-items.js"
import { filterCommandCenterItems } from "./command-center-search.js"
import type { CommandCenterContext } from "./command-center-context.js"
import { buildCommandCenterRootItems } from "./command-center-root-items.js"
import { buildScopedCommandCenterItems } from "./command-center-scoped-items.js"
import { COMMAND_TREE } from "./command-center-tree.js"
import type { CommandCenterItem } from "./command-center-types.js"

export type { CommandCenterItem } from "./command-center-types.js"

export function buildCommandCenterItems(input: string, context: CommandCenterContext): CommandCenterItem[] {
  if (!input.startsWith("/")) {
    return []
  }

  const normalized = input.trimStart()
  if (!normalized.startsWith("/")) {
    return []
  }

  if (normalized === "/") {
    return buildCommandCenterRootItems(context)
  }

  if (normalized.startsWith("/provider ")) {
    return buildProviderItems(normalized, COMMAND_TREE.find((node) => node.id === "provider")!, context)
  }

  if (normalized.startsWith("/model ")) {
    return buildModelItems(normalized, context)
  }

  if (normalized.startsWith("/variant ")) {
    return buildVariantItems(normalized, context)
  }

  if (normalized.startsWith("/view ")) {
    return buildViewItems(normalized)
  }

  if (context.focusedProvider && providerSupportsNamespaceCommands(context.focusedProvider)) {
    const namespace = `${providerNamespace(context.focusedProvider)} `
    if (normalized === providerNamespace(context.focusedProvider) || normalized.startsWith(namespace)) {
      return buildProviderNamespaceItems(normalized, context.focusedProvider, context.providerCommandCatalogs)
    }
  }

  const scoped = buildScopedCommandCenterItems(normalized, context)
  if (scoped) {
    return scoped
  }

  return filterCommandCenterItems(buildCommandCenterRootItems(context), normalized.slice(1).toLowerCase())
}
