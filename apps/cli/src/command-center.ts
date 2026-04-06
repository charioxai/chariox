import { catalogModelOptions, type BackendProviderId, type ProviderCatalog } from "./provider-catalog.js"
import {
  type ProviderCommandCatalogs,
  providerNamespace,
  providerNamespaceDescription,
} from "./provider-command-catalog.js"

export type CommandCenterItem = {
  id: string
  label: string
  description: string
  kind: "command" | "group" | "provider" | "model" | "variant"
  value: string
  searchAliases?: string[] | undefined
}

type CommandContext = {
  providerCatalog: ProviderCatalog
  providerCommandCatalogs: ProviderCommandCatalogs
  currentProvider: BackendProviderId
  focusedProvider: BackendProviderId | null
  currentModel: string
  currentVariant: string
}

type CommandNode = {
  id: string
  label: string
  description: string
  value: string
  children?: CommandNode[]
}

const COMMAND_TREE: CommandNode[] = [
  {
    id: "session",
    label: "/session",
    description: "Create, attach, list, or delete sessions",
    value: "/session ",
    children: [
      { id: "session-new", label: "new", description: "Create and attach a new session", value: "/session new " },
      { id: "session-attach", label: "attach", description: "Attach to an existing session", value: "/session attach " },
      { id: "session-list", label: "list", description: "List available sessions", value: "/session list" },
      { id: "session-delete", label: "delete", description: "Delete the current or referenced session", value: "/session delete " },
    ],
  },
  {
    id: "agent",
    label: "/agent",
    description: "Manage agents in the current session",
    value: "/agent ",
    children: [
      { id: "agent-spawn", label: "spawn", description: "Spawn a new agent in the session", value: "/agent spawn " },
      { id: "agent-delete", label: "delete", description: "Delete the focused or named agent", value: "/agent delete " },
      { id: "agent-destroy", label: "destroy", description: "Alias for /agent delete", value: "/agent destroy " },
      { id: "agent-focus", label: "focus", description: "Focus on a specific agent", value: "/agent focus " },
      { id: "agent-list", label: "list", description: "List all agents in the session", value: "/agent list" },
      { id: "agent-cycle", label: "cycle", description: "Cycle to next agent", value: "/agent cycle" },
    ],
  },
  {
    id: "workflow",
    label: "/workflow",
    description: "Inspect, edit, and run workflows",
    value: "/workflow ",
    children: [
      { id: "workflow-open", label: "open", description: "Open the workflow outline", value: "/workflow" },
      { id: "workflow-list", label: "list", description: "List workflows in the workspace", value: "/workflow list" },
      { id: "workflow-show", label: "show", description: "Show a workflow by id or alias", value: "/workflow show " },
      { id: "workflow-new", label: "new", description: "Create a new workflow", value: "/workflow new " },
      { id: "workflow-run", label: "run", description: "Invoke a workflow endpoint", value: "/workflow run " },
      { id: "workflow-start", label: "start", description: "Alias for /workflow run", value: "/workflow start " },
      { id: "workflow-max-turns", label: "max-turns", description: "Set max workflow turns across all agents", value: "/workflow max-turns " },
      { id: "workflow-runs", label: "runs", description: "List workflow runs in the session", value: "/workflow runs " },
      { id: "workflow-cancel", label: "cancel", description: "Cancel a workflow run", value: "/workflow cancel " },
      { id: "workflow-resume", label: "resume", description: "Resume a stopped workflow run", value: "/workflow resume " },
      { id: "workflow-terminal", label: "terminal", description: "Show the shared workflow console in the I/O panel", value: "/workflow terminal " },
      { id: "workflow-alias", label: "alias", description: "Assign an alias to an existing workflow", value: "/workflow " },
      {
        id: "workflow-node",
        label: "node",
        description: "Manage workflow nodes",
        value: "/workflow node ",
        children: [
          { id: "workflow-node-add", label: "add", description: "Add a workflow node for an agent", value: "/workflow node add " },
          { id: "workflow-node-remove", label: "remove", description: "Remove a workflow node", value: "/workflow node remove " },
          {
            id: "workflow-node-instructions",
            label: "instructions",
            description: "Manage workflow node instructions",
            value: "/workflow node instructions ",
            children: [
              { id: "workflow-node-instructions-show", label: "show", description: "Show workflow node instructions in the I/O panel", value: "/workflow node instructions show " },
              { id: "workflow-node-instructions-set", label: "set", description: "Set workflow node instructions (optional file)", value: "/workflow node instructions set " },
              { id: "workflow-node-instructions-save", label: "save", description: "Save the current workflow node instructions draft", value: "/workflow node instructions save" },
              { id: "workflow-node-instructions-close", label: "close", description: "Close the workflow node instructions panel", value: "/workflow node instructions close" },
            ],
          },
        ],
      },
      {
        id: "workflow-edge",
        label: "edge",
        description: "Manage workflow edges",
        value: "/workflow edge ",
        children: [
          { id: "workflow-edge-shorthand", label: "connect", description: "Add a directed edge between two workflow nodes", value: "/workflow " },
          { id: "workflow-edge-add", label: "add", description: "Add a directed edge between workflow nodes", value: "/workflow edge add " },
          { id: "workflow-edge-remove", label: "remove", description: "Remove a workflow edge", value: "/workflow edge remove " },
        ],
      },
      {
        id: "workflow-endpoint",
        label: "endpoint",
        description: "Manage workflow endpoints",
        value: "/workflow endpoint ",
        children: [
          { id: "workflow-endpoint-new", label: "new", description: "Create a workflow endpoint", value: "/workflow endpoint new " },
          { id: "workflow-endpoint-alias", label: "alias", description: "Assign an alias to a workflow endpoint", value: "/workflow endpoint alias " },
          { id: "workflow-endpoint-bind", label: "bind", description: "Bind an endpoint to a workflow node", value: "/workflow endpoint bind " },
        ],
      },
    ],
  },
  {
    id: "provider",
    label: "/provider",
    description: "Change provider and manage auth",
    value: "/provider ",
    children: [
      { id: "provider-select", label: "switch", description: "Choose the active provider", value: "/provider " },
      { id: "provider-status", label: "status", description: "Show provider auth status", value: "/provider status " },
      { id: "provider-login", label: "login", description: "Start provider login", value: "/provider login " },
      { id: "provider-logout", label: "logout", description: "Log out a provider", value: "/provider logout " },
      { id: "provider-reauth", label: "reauth", description: "Reauthenticate a provider", value: "/provider reauth " },
    ],
  },
  {
    id: "model",
    label: "/model",
    description: "Change the active model",
    value: "/model ",
  },
  {
    id: "variant",
    label: "/variant",
    description: "Change the active model variant",
    value: "/variant ",
  },
  {
    id: "view",
    label: "/view",
    description: "Change multi-agent response layout",
    value: "/view ",
    children: [
      { id: "view-individual", label: "individual", description: "Show one focused agent transcript at a time", value: "/view individual" },
      { id: "view-split", label: "split", description: "Split the response area across active session agents", value: "/view split" },
    ],
  },
  {
    id: "misc",
    label: "/misc",
    description: "Attachments, control, and app commands",
    value: "/",
    children: [
      { id: "attach", label: "/attach", description: "Attach files or screenshots to the prompt", value: "/attach " },
      { id: "stop", label: "/stop", description: "Stop the active provider turn", value: "/stop" },
      { id: "waiting", label: "/waiting", description: "Go to the waiting room", value: "/waiting" },
      { id: "exit", label: "/exit", description: "Leave the CLI", value: "/exit" },
    ],
  },
]

export function buildCommandCenterItems(input: string, context: CommandContext): CommandCenterItem[] {
  if (!input.startsWith("/")) {
    return []
  }

  const normalized = input.trimStart()
  if (!normalized.startsWith("/")) {
    return []
  }

  if (normalized === "/") {
    return rootItems(context)
  }

  if (normalized.startsWith("/provider ")) {
    return buildProviderItems(normalized, context)
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

  if (context.focusedProvider) {
    const namespace = `${providerNamespace(context.focusedProvider)} `
    if (normalized === providerNamespace(context.focusedProvider) || normalized.startsWith(namespace)) {
      return buildProviderNamespaceItems(normalized, context.focusedProvider, context.providerCommandCatalogs)
    }
  }

  const scoped = buildScopedCommandItems(normalized, context)
  if (scoped) {
    return scoped
  }

  return filterCommandCenterItems(rootItems(context), normalized.slice(1).toLowerCase())
}

export function shouldSubmitExactCommandCenterMatch(item: CommandCenterItem, currentPrompt: string) {
  if (item.kind !== "command" || !item.value.endsWith(" ")) {
    return false
  }
  return currentPrompt.startsWith(item.value) || currentPrompt === item.value.trim()
}

function rootItems(context: CommandContext) {
  const rootNodes = COMMAND_TREE
    .filter((node) => node.id !== "misc")
    .map((node) => mapRootGroup(node))
  if (context.focusedProvider) {
    const catalog = context.providerCommandCatalogs[context.focusedProvider] ?? emptyProviderCommandCatalog(context.focusedProvider)
    rootNodes.push({
      id: `provider-namespace-${context.focusedProvider}`,
      label: providerNamespace(context.focusedProvider),
      description: providerNamespaceDescription(context.focusedProvider, catalog.commands.length),
      kind: "group",
      value: `${providerNamespace(context.focusedProvider)} `,
      ...(catalog.commands.length > 0
        ? {
          searchAliases: catalog.commands.flatMap((command) => [command.name, command.description, command.value]),
        }
        : {}),
    })
  }
  const miscNodes = COMMAND_TREE.find((node) => node.id === "misc")?.children?.map(mapNodeToItem) ?? []
  return [...rootNodes, ...miscNodes]
}

function buildProviderNamespaceItems(
  input: string,
  provider: BackendProviderId,
  catalogs: ProviderCommandCatalogs,
) {
  const catalog = catalogs[provider] ?? emptyProviderCommandCatalog(provider)
  const namespace = providerNamespace(provider)
  const rootItem: CommandCenterItem = {
    id: `provider-namespace-${provider}`,
    label: namespace,
    description: catalog.commands.length > 0
      ? providerNamespaceDescription(provider, catalog.commands.length)
      : `${providerNamespaceDescription(provider, 0)}; no registered completions for this provider version yet`,
    kind: "group",
    value: `${namespace} `,
  }
  if (input === namespace || input === `${namespace} `) {
    return catalog.commands.length > 0
      ? catalog.commands.map((command) => ({
        id: `${provider}-${command.id}`,
        label: command.name,
        description: command.description,
        kind: "command" as const,
        value: `${namespace} ${command.value}`,
      }))
      : [rootItem]
  }
  const query = input.slice(`${namespace} `.length).trim().toLowerCase()
  if (catalog.commands.length === 0) {
    return filterCommandCenterItems([rootItem], query)
  }
  return filterCommandCenterItems(
    catalog.commands.map((command) => ({
      id: `${provider}-${command.id}`,
      label: command.name,
      description: command.description,
      kind: "command" as const,
      value: `${namespace} ${command.value}`,
    })),
    query,
  )
}

function buildProviderItems(input: string, context: CommandContext) {
  const query = input.slice("/provider ".length).trim().toLowerCase()
  return filterCommandCenterItems([
    mapNodeToItem(COMMAND_TREE.find((node) => node.id === "provider")!),
    {
      id: "provider-opencode",
      label: "OpenCode",
      description: "Use the OpenCode backend",
      kind: "provider",
      value: "opencode",
    },
    {
      id: "provider-codex",
      label: "Codex",
      description: "Use the Codex backend",
      kind: "provider",
      value: "codex",
    },
    {
      id: "provider-status",
      label: "status",
      description: "Show auth status for the current or named provider",
      kind: "command",
      value: "/provider status ",
    },
    {
      id: "provider-login",
      label: "login",
      description: "Start login for the current or named provider",
      kind: "command",
      value: "/provider login ",
    },
    {
      id: "provider-logout",
      label: "logout",
      description: "Log out the current or named provider",
      kind: "command",
      value: "/provider logout ",
    },
    {
      id: "provider-reauth",
      label: "reauth",
      description: "Log out and start a fresh login for the current or named provider",
      kind: "command",
      value: "/provider reauth ",
    },
  ], query)
}

function buildModelItems(input: string, context: CommandContext) {
  const query = input.slice("/model ".length).trim().toLowerCase()
  return filterCommandCenterItems(
    catalogModelOptions(context.providerCatalog, context.currentProvider).map((option) => ({
      id: `model-${option.id}`,
      label: `${option.providerName} ${option.label}`,
      description: option.id === context.currentModel ? "current model" : option.id,
      kind: "model" as const,
      value: option.id,
    })),
    query,
  )
}

function buildVariantItems(input: string, context: CommandContext) {
  const query = input.slice("/variant ".length).trim().toLowerCase()
  const current = catalogModelOptions(context.providerCatalog, context.currentProvider).find((option) => option.id === context.currentModel)
  const variants = current?.variants ?? []
  return filterCommandCenterItems(
    variants.map((variant) => ({
      id: `variant-${variant}`,
      label: variant,
      description: `${current?.label ?? context.currentModel}${variant === context.currentVariant ? " • current" : ""}`,
      kind: "variant" as const,
      value: variant,
    })),
    query,
  )
}

function buildViewItems(input: string) {
  const query = input.slice("/view ".length).trim().toLowerCase()
  return filterCommandCenterItems([
    {
      id: "view-individual",
      label: "individual",
      description: "Show one focused agent transcript at a time",
      kind: "command",
      value: "/view individual",
    },
    {
      id: "view-split",
      label: "split",
      description: "Split the response area across active session agents",
      kind: "command",
      value: "/view split",
    },
  ], query)
}

function buildScopedCommandItems(input: string, context: CommandContext) {
  const scopeNodes = context.focusedProvider
    ? [
      ...COMMAND_TREE,
      {
        id: `provider-namespace-${context.focusedProvider}`,
        label: providerNamespace(context.focusedProvider),
        description: providerNamespaceDescription(
          context.focusedProvider,
          (context.providerCommandCatalogs[context.focusedProvider] ?? emptyProviderCommandCatalog(context.focusedProvider)).commands.length,
        ),
        value: `${providerNamespace(context.focusedProvider)} `,
      },
    ]
    : COMMAND_TREE
  const nodes = collectCommandNodes(scopeNodes)
  const exactCommand = nodes.find((node) => !node.children?.length && input === node.value)
  if (exactCommand && input.endsWith(" ")) {
    return []
  }

  const scope = findDeepestScope(input, scopeNodes.filter((node) => node.value !== "/"))
  if (!scope) {
    return null
  }

  const query = input.slice(scope.node.value.length).trim().toLowerCase()
  if (scope.node.children?.length) {
    const items = [mapNodeToItem(scope.node), ...scope.node.children.map(mapNodeToItem)]
    return query ? filterCommandCenterItems(items, query) : items
  }
  return null
}

function emptyProviderCommandCatalog(provider: BackendProviderId) {
  return {
    provider,
    source: "shipped" as const,
    discovery: "none" as const,
    commands: [],
  }
}

function findDeepestScope(input: string, nodes: CommandNode[]): { node: CommandNode } | null {
  let match: CommandNode | null = null
  for (const node of nodes) {
    if (input === node.value.trim() || input === node.value || input.startsWith(node.value)) {
      if (!match || node.value.length > match.value.length) {
        match = node
      }
    }
    if (node.children?.length) {
      const childMatch = findDeepestScope(input, node.children)
      if (childMatch && (!match || childMatch.node.value.length > match.value.length)) {
        match = childMatch.node
      }
    }
  }
  return match ? { node: match } : null
}

function collectCommandNodes(nodes: CommandNode[]): CommandNode[] {
  return nodes.flatMap((node) => [node, ...collectCommandNodes(node.children ?? [])])
}

function mapNodeToItem(node: CommandNode): CommandCenterItem {
  return {
    id: node.id,
    label: node.label,
    description: node.children?.length ? `${node.description} (${node.children.length})` : node.description,
    kind: node.children?.length ? "group" : "command",
    value: node.value,
    ...(node.children?.length ? { searchAliases: collectDescendantSearchAliases(node) } : {}),
  }
}

function mapRootGroup(node: CommandNode): CommandCenterItem {
  return {
    id: node.id,
    label: node.label,
    description: node.children?.length ? `${node.description} (${node.children.length})` : node.description,
    kind: "group",
    value: node.value,
    ...(node.children?.length ? { searchAliases: collectDescendantSearchAliases(node) } : {}),
  }
}

function filterCommandCenterItems(items: CommandCenterItem[], query: string) {
  if (!query) {
    return items
  }
  return items
    .map((item) => ({ item, score: scoreCommandCenterItem(item, query) }))
    .filter((entry) => entry.score > 0)
    .sort((left, right) => right.score - left.score || left.item.label.localeCompare(right.item.label))
    .map((entry) => entry.item)
    .slice(0, 20)
}

function scoreCommandCenterItem(item: CommandCenterItem, query: string) {
  const haystacks = [item.label.toLowerCase(), item.description.toLowerCase(), item.value.toLowerCase()]
  let score = 0
  for (const haystack of haystacks) {
    if (haystack.startsWith(query) || haystack.startsWith(`/${query}`)) {
      score = Math.max(score, 4)
    } else if (haystack.includes(query)) {
      score = Math.max(score, 2)
    }
  }
  for (const alias of item.searchAliases ?? []) {
    const haystack = alias.toLowerCase()
    if (haystack.startsWith(query) || haystack.startsWith(`/${query}`)) {
      score = Math.max(score, 3)
    } else if (haystack.includes(query)) {
      score = Math.max(score, 1)
    }
  }
  return score
}

function collectDescendantSearchAliases(node: CommandNode): string[] {
  const aliases = new Set<string>()
  for (const child of node.children ?? []) {
    aliases.add(child.label)
    aliases.add(child.value.trim())
    aliases.add(child.description)
    for (const alias of collectDescendantSearchAliases(child)) {
      aliases.add(alias)
    }
  }
  return [...aliases]
}
