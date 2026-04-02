import { catalogModelOptions, type ProviderCatalog } from "./provider-catalog.js"

export type CommandCenterItem = {
  id: string
  label: string
  description: string
  kind: "command" | "provider" | "model" | "variant"
  value: string
}

type CommandContext = {
  providerCatalog: ProviderCatalog
  currentModel: string
  currentVariant: string
}

const ROOT_COMMANDS: CommandCenterItem[] = [
  { id: "session-new", label: "/session new", description: "Create and attach a new session", kind: "command", value: "/session new " },
  { id: "session-attach", label: "/session attach", description: "Attach to an existing session", kind: "command", value: "/session attach " },
  { id: "session-list", label: "/session list", description: "List available sessions", kind: "command", value: "/session list" },
  { id: "session-delete", label: "/session delete", description: "Delete the current or referenced session", kind: "command", value: "/session delete " },
  { id: "attach", label: "/attach", description: "Attach files or screenshots to the prompt", kind: "command", value: "/attach " },
  { id: "exit", label: "/exit", description: "Leave the CLI", kind: "command", value: "/exit" },
  { id: "waiting", label: "/waiting", description: "Go to the waiting room", kind: "command", value: "/waiting" },
  { id: "stop", label: "/stop", description: "Stop the active provider turn", kind: "command", value: "/stop" },
  { id: "provider", label: "/provider", description: "Change provider", kind: "command", value: "/provider " },
  { id: "model", label: "/model", description: "Change model", kind: "command", value: "/model " },
  { id: "variant", label: "/variant", description: "Change model variant", kind: "command", value: "/variant " },
  { id: "view", label: "/view", description: "Change multi-agent response layout", kind: "command", value: "/view " },
  { id: "agent-spawn", label: "/agent spawn", description: "Spawn a new agent in the session", kind: "command", value: "/agent spawn " },
  { id: "agent-delete", label: "/agent delete", description: "Delete the focused or named agent", kind: "command", value: "/agent delete " },
  { id: "agent-destroy", label: "/agent destroy", description: "Alias for /agent delete", kind: "command", value: "/agent destroy " },
  { id: "agent-focus", label: "/agent focus", description: "Focus on a specific agent", kind: "command", value: "/agent focus " },
  { id: "agent-list", label: "/agent list", description: "List all agents in the session", kind: "command", value: "/agent list" },
  { id: "agent-cycle", label: "/agent cycle", description: "Cycle to next agent", kind: "command", value: "/agent cycle" },
  { id: "workflow", label: "/workflow", description: "Open the workflow canvas", kind: "command", value: "/workflow" },
  { id: "workflow-new", label: "/workflow new", description: "Create a new workflow", kind: "command", value: "/workflow new " },
]

export function buildCommandCenterItems(input: string, context: CommandContext): CommandCenterItem[] {
  if (!input.startsWith("/")) {
    return []
  }

  const normalized = input.trimStart()
  if (!normalized.startsWith("/")) {
    return []
  }

  const [command, ...restParts] = normalized.split(/\s+/)
  const query = restParts.join(" ").trim().toLowerCase()

  if ((command === "/provider" || command === "/provider ") && normalized.startsWith("/provider ")) {
    return filterCommandCenterItems([
      {
        id: "provider-opencode",
        label: "OpenCode",
        description: "Use the OpenCode backend",
        kind: "provider",
        value: "opencode",
      },
    ], query)
  }

  if ((command === "/model" || command === "/model ") && normalized.startsWith("/model ")) {
    return filterCommandCenterItems(
      catalogModelOptions(context.providerCatalog).map((option) => ({
        id: `model-${option.id}`,
        label: `${option.providerName} ${option.label}`,
        description: option.id === context.currentModel ? "current model" : option.id,
        kind: "model",
        value: option.id,
      })),
      query,
    )
  }

  if ((command === "/variant" || command === "/variant ") && normalized.startsWith("/variant ")) {
    const current = catalogModelOptions(context.providerCatalog).find((option) => option.id === context.currentModel)
    const variants = current?.variants ?? []
    return filterCommandCenterItems(
      variants.map((variant) => ({
        id: `variant-${variant}`,
        label: variant,
        description: `${current?.label ?? context.currentModel}${variant === context.currentVariant ? " • current" : ""}`,
        kind: "variant",
        value: variant,
      })),
      query,
    )
  }

  if ((command === "/view" || command === "/view ") && normalized.startsWith("/view ")) {
    return filterCommandCenterItems(
      [
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
      ],
      query,
    )
  }

  if (ROOT_COMMANDS.some((item) => item.kind === "command" && item.value.endsWith(" ") && normalized === item.value)) {
    return []
  }

  return filterCommandCenterItems(ROOT_COMMANDS, normalized.slice(1).toLowerCase())
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
    .slice(0, 10)
}

function scoreCommandCenterItem(item: CommandCenterItem, query: string) {
  const haystacks = [item.label.toLowerCase(), item.description.toLowerCase(), item.value.toLowerCase()]
  let score = 0
  for (const haystack of haystacks) {
    if (haystack.startsWith(query) || haystack.startsWith(`/${query}`)) {
      score = Math.max(score, 3)
    } else if (haystack.includes(query)) {
      score = Math.max(score, 1)
    }
  }
  return score
}
