import type { CommandCenterWorkflowRegistryEntry } from "./command-center-context.js"

export function workflowRegistrySuggestionEntriesFromResponse(
  response: Record<string, unknown>,
): CommandCenterWorkflowRegistryEntry[] {
  const payload = response.WorkflowRegistryListed
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return []
  }
  const entries = (payload as Record<string, unknown>).entries
  if (!Array.isArray(entries)) {
    return []
  }
  return entries.flatMap((entry) => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      return []
    }
    const record = entry as Record<string, unknown>
    const name = stringField(record, "name")
    const sourceScope = workflowRegistrySourceScope(record.source_scope)
    const sourceKind = workflowRegistrySourceKind(record.source_kind)
    if (!name || !sourceScope || !sourceKind) {
      return []
    }
    const suggestion: CommandCenterWorkflowRegistryEntry = {
      name,
      sourceScope,
      sourceKind,
    }
    const summary = objectField(record, "summary")
    const endpoints = summary ? stringArrayField(summary, "endpoints") : undefined
    const queues = summary ? stringArrayField(summary, "queues") : undefined
    if (endpoints) {
      suggestion.endpoints = endpoints
    }
    if (queues) {
      suggestion.queues = queues
    }
    return [suggestion]
  })
}

function stringField(record: Record<string, unknown>, key: string): string | null {
  const value = record[key]
  return typeof value === "string" && value.length > 0 ? value : null
}

function objectField(record: Record<string, unknown>, key: string): Record<string, unknown> | null {
  const value = record[key]
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null
}

function stringArrayField(record: Record<string, unknown>, key: string): readonly string[] | undefined {
  const value = record[key]
  if (!Array.isArray(value)) {
    return undefined
  }
  const strings = value.filter((item): item is string => typeof item === "string" && item.length > 0)
  return strings.length ? strings : undefined
}

function workflowRegistrySourceScope(value: unknown): CommandCenterWorkflowRegistryEntry["sourceScope"] | null {
  return value === "workspace" || value === "user" || value === "builtin" ? value : null
}

function workflowRegistrySourceKind(value: unknown): CommandCenterWorkflowRegistryEntry["sourceKind"] | null {
  return value === "single_file" || value === "source_directory" ? value : null
}
