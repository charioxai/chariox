import { filterCommandCenterItems } from "./command-center-search.js"
import type {
  CommandCenterContext,
  CommandCenterWorkflowRegistryEntry,
} from "./command-center-context.js"
import type { CommandCenterItem } from "./command-center-types.js"

const LOAD_PREFIX = "/workflow load "
const RUN_PREFIX = "/workflow run "
const GET_PREFIX = "/workflow registry get "
const DELETE_PREFIX = "/workflow registry delete "

export function buildWorkflowRegistryItems(
  input: string,
  context: Pick<CommandCenterContext, "workflowRegistryEntries">,
): CommandCenterItem[] | null {
  const entries = [...(context.workflowRegistryEntries ?? [])].sort((left, right) => left.name.localeCompare(right.name))
  if (entries.length === 0) {
    return null
  }

  if (input.startsWith(LOAD_PREFIX)) {
    return workflowNameItems(entries, input.slice(LOAD_PREFIX.length), "load")
  }
  if (input.startsWith(GET_PREFIX)) {
    return workflowNameItems(entries, input.slice(GET_PREFIX.length), "get")
  }
  if (input.startsWith(DELETE_PREFIX)) {
    return workflowNameItems(entries.filter((entry) => entry.sourceScope !== "builtin"), input.slice(DELETE_PREFIX.length), "delete")
  }
  if (input.startsWith(RUN_PREFIX)) {
    return workflowRunItems(entries, input.slice(RUN_PREFIX.length))
  }
  return null
}

function workflowNameItems(
  entries: readonly CommandCenterWorkflowRegistryEntry[],
  rawQuery: string,
  action: "load" | "get" | "delete",
): CommandCenterItem[] {
  const query = rawQuery.trim().toLowerCase()
  const prefix = action === "load"
    ? LOAD_PREFIX
    : action === "get"
      ? GET_PREFIX
      : DELETE_PREFIX
  return filterCommandCenterItems(entries.map((entry) => ({
    id: `workflow-registry-${action}-${entry.sourceScope}-${entry.name}`,
    label: entry.name,
    description: registryEntryDescription(entry),
    kind: "command" as const,
    value: `${prefix}${entry.name}`,
    searchAliases: [entry.sourceScope, entry.sourceKind],
  })), query)
}

function workflowRunItems(
  entries: readonly CommandCenterWorkflowRegistryEntry[],
  rest: string,
): CommandCenterItem[] {
  const endpointMatch = rest.match(/^(\S+)\s+.*--endpoint\s+(\S*)$/)
  if (endpointMatch) {
    const entry = entries.find((candidate) => candidate.name === endpointMatch[1])
    return entry ? endpointItems(entry, endpointMatch[2] ?? "") : []
  }

  const queueMatch = rest.match(/^(\S+)\s+.*--queue\s+(\S*)$/)
  if (queueMatch) {
    const entry = entries.find((candidate) => candidate.name === queueMatch[1])
    return entry ? queueItems(entry, queueMatch[2] ?? "") : []
  }

  const exactEntry = entries.find((entry) => rest === entry.name || rest === `${entry.name} `)
  if (exactEntry) {
    return endpointItems(exactEntry, "")
  }

  const query = rest.trim().toLowerCase()
  return filterCommandCenterItems(entries.map((entry) => {
    const endpoint = defaultEndpoint(entry)
    return {
      id: `workflow-registry-run-${entry.sourceScope}-${entry.name}`,
      label: entry.name,
      description: registryEntryDescription(entry),
      kind: "group" as const,
      value: `${RUN_PREFIX}${entry.name} --endpoint ${endpoint} --prompt "" `,
      searchAliases: [entry.sourceScope, entry.sourceKind, endpoint],
    }
  }), query)
}

function endpointItems(entry: CommandCenterWorkflowRegistryEntry, rawQuery: string): CommandCenterItem[] {
  const query = rawQuery.trim().toLowerCase()
  return filterCommandCenterItems(endpointHandles(entry).map((endpoint) => ({
    id: `workflow-registry-run-${entry.name}-endpoint-${endpoint}`,
    label: endpoint,
    description: `${registryEntryDescription(entry)} · endpoint ${endpoint}`,
    kind: "group" as const,
    value: `${RUN_PREFIX}${entry.name} --endpoint ${endpoint} --prompt "" `,
  })), query)
}

function queueItems(entry: CommandCenterWorkflowRegistryEntry, rawQuery: string): CommandCenterItem[] {
  const queues = entry.queues ?? []
  if (queues.length === 0) {
    return []
  }
  const query = rawQuery.trim().toLowerCase()
  const endpoint = defaultEndpoint(entry)
  return filterCommandCenterItems(queues.map((queue) => ({
    id: `workflow-registry-run-${entry.name}-queue-${queue}`,
    label: queue,
    description: `${registryEntryDescription(entry)} · queue ${queue}`,
    kind: "group" as const,
    value: `${RUN_PREFIX}${entry.name} --endpoint ${endpoint} --queue ${queue} --prompt "" `,
  })), query)
}

function defaultEndpoint(entry: CommandCenterWorkflowRegistryEntry): string {
  const endpoints = endpointHandles(entry)
  return endpoints.includes("entry") ? "entry" : endpoints[0] ?? "entry"
}

function endpointHandles(entry: CommandCenterWorkflowRegistryEntry): readonly string[] {
  return entry.endpoints?.length ? entry.endpoints : ["entry"]
}

function registryEntryDescription(entry: CommandCenterWorkflowRegistryEntry): string {
  return `${entry.sourceScope} · ${entry.sourceKind}`
}
