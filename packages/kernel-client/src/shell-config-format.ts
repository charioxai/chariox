import type {
  CharioxUserConfigPayload,
  UserConfigSchemaEntry,
} from "./kernel-types.js"

export function formatConfigSchemaKeys(entries: UserConfigSchemaEntry[]): string {
  if (entries.length === 0) {
    return "(no config keys)"
  }
  return entries
    .filter((entry) => entry.settable)
    .map((entry) => {
      const values = entry.allowed_values && entry.allowed_values.length > 0
        ? ` values=${entry.allowed_values.join("|")}`
        : ""
      const unset = entry.unsettable ? " unset" : ""
      return `${entry.path} (${entry.value_type}; ${entry.status}; ${entry.effect}${unset}${values})`
    })
    .join("\n")
}

export function configMutationMessage(prefix: string, payload: CharioxUserConfigPayload): string {
  const effects = payload.effects ?? []
  if (effects.length === 0) {
    return prefix
  }
  return [prefix, ...effects.map((effect) => effect.message)].join("\n")
}
