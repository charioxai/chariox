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
import { buildWorkflowRegistryItems } from "./command-center-workflow-registry-items.js"
import type { CommandCenterItem } from "./command-center-types.js"

export type { CommandCenterItem } from "./command-center-types.js"

export function buildCommandCenterItems(input: string, context: CommandCenterContext): CommandCenterItem[] {
  const agentItems = buildAgentAliasItems(input, context.agentAliases ?? [])
  if (agentItems) {
    return agentItems
  }
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
    const providerNode = context.commandTree.find((node) => node.id === "provider")
    return providerNode ? buildProviderItems(normalized, providerNode, context) : []
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

  const registryItems = buildWorkflowRegistryItems(normalized, context)
  if (registryItems) {
    return registryItems
  }

  const scoped = buildScopedCommandCenterItems(normalized, context)
  if (scoped) {
    return scoped
  }

  return filterCommandCenterItems(buildCommandCenterRootItems(context), normalized.slice(1).toLowerCase())
}

function buildAgentAliasItems(
  input: string,
  aliases: readonly string[],
): CommandCenterItem[] | null {
  if (!input.startsWith("@")) {
    return null
  }
  const match = input.match(/^@([^\s]*)$/)
  if (!match) {
    return []
  }
  const query = (match[1] ?? "").toLowerCase()
  return [...new Map(aliases
    .map((alias) => alias.trim())
    .filter((alias) => alias && !/\s/.test(alias))
    .map((alias) => [alias.toLowerCase(), alias])).values()]
    .filter((alias) => alias.toLowerCase().includes(query))
    .sort((left, right) => left.localeCompare(right))
    .map((alias) => ({
      id: `agent-${alias.toLowerCase()}`,
      label: `@${alias}`,
      description: `Send the prompt to ${alias} and focus its pane`,
      kind: "agent" as const,
      value: alias,
    }))
}
