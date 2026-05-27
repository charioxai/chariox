import type {
  ArrobaUserConfigPayload,
  ArrobaUserConfigSchemaPayload,
  UserConfigSchemaEntry,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"

type FooterTone = "info" | "error"

export type ConfigCommandHandlerDeps = {
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  getUserConfig?: () => Promise<ArrobaUserConfigPayload>
  getUserConfigSchema?: () => Promise<ArrobaUserConfigSchemaPayload>
  setUserConfigValue?: (path: string, value: string) => Promise<ArrobaUserConfigPayload>
  unsetUserConfigValue?: (path: string) => Promise<ArrobaUserConfigPayload>
}

export async function handleConfigSlashCommand(
  deps: ConfigCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "config" }>,
): Promise<void> {
  const [subcommand, keyPath, ...rest] = command.args
  if (!subcommand || subcommand === "show") {
    await showUserConfig(deps)
    return
  }
  if (subcommand === "path") {
    await showUserConfigPath(deps)
    return
  }
  if (subcommand === "keys" || subcommand === "list") {
    await listUserConfigKeys(deps)
    return
  }
  if (subcommand === "schema") {
    await showUserConfigSchema(deps)
    return
  }
  if (subcommand === "set") {
    await setUserConfigValue(deps, keyPath, rest)
    return
  }
  if (subcommand === "unset") {
    await unsetUserConfigValue(deps, keyPath)
    return
  }
  if (subcommand === "workspace-live-sync") {
    await setWorkspaceLiveSyncMode(deps, keyPath, rest)
    return
  }
  deps.flashFooter(
    "usage: /config show | path | keys | schema | set <path> <value> | unset <path> | workspace-live-sync required|unrestricted",
    "error",
  )
}

function appendUserConfigEffects(
  deps: ConfigCommandHandlerDeps,
  payload: ArrobaUserConfigPayload,
): void {
  const effects = payload.effects ?? []
  if (effects.length > 0) {
    deps.appendNotice(effects.map((effect) => effect.message).join("\n"))
  }
}

function formatConfigSchemaKeys(entries: UserConfigSchemaEntry[]): string {
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

async function showUserConfig(deps: ConfigCommandHandlerDeps): Promise<void> {
  if (!deps.getUserConfig) {
    deps.flashFooter("user config is unavailable in this build", "error")
    return
  }
  const payload = await deps.getUserConfig()
  deps.appendNotice(`config path: ${payload.path}\n${JSON.stringify(payload.config, null, 2)}`)
  deps.flashFooter(`config loaded from ${payload.path}`, "info")
}

async function showUserConfigPath(deps: ConfigCommandHandlerDeps): Promise<void> {
  if (!deps.getUserConfig) {
    deps.flashFooter("user config is unavailable in this build", "error")
    return
  }
  const payload = await deps.getUserConfig()
  deps.appendNotice(payload.path)
  deps.flashFooter(`config path: ${payload.path}`, "info")
}

async function listUserConfigKeys(deps: ConfigCommandHandlerDeps): Promise<void> {
  if (!deps.getUserConfigSchema) {
    deps.flashFooter("user config schema is unavailable in this build", "error")
    return
  }
  const payload = await deps.getUserConfigSchema()
  deps.appendNotice(formatConfigSchemaKeys(payload.entries))
  deps.flashFooter(`listed ${payload.entries.length} config key${payload.entries.length === 1 ? "" : "s"}`, "info")
}

async function showUserConfigSchema(deps: ConfigCommandHandlerDeps): Promise<void> {
  if (!deps.getUserConfigSchema) {
    deps.flashFooter("user config schema is unavailable in this build", "error")
    return
  }
  const payload = await deps.getUserConfigSchema()
  deps.appendNotice(JSON.stringify(payload.entries, null, 2))
  deps.flashFooter(`listed ${payload.entries.length} config schema entr${payload.entries.length === 1 ? "y" : "ies"}`, "info")
}

async function setUserConfigValue(
  deps: ConfigCommandHandlerDeps,
  keyPath: string | undefined,
  rest: string[],
): Promise<void> {
  if (!deps.setUserConfigValue) {
    deps.flashFooter("user config updates are unavailable in this build", "error")
    return
  }
  const value = rest.join(" ").trim()
  if (!keyPath || !value) {
    deps.flashFooter("usage: /config set <path> <value>", "error")
    return
  }
  const payload = await deps.setUserConfigValue(keyPath, value)
  appendUserConfigEffects(deps, payload)
  deps.flashFooter(`config ${keyPath} set to ${value}`, "info")
}

async function unsetUserConfigValue(
  deps: ConfigCommandHandlerDeps,
  keyPath: string | undefined,
): Promise<void> {
  if (!deps.unsetUserConfigValue) {
    deps.flashFooter("user config updates are unavailable in this build", "error")
    return
  }
  if (!keyPath) {
    deps.flashFooter("usage: /config unset <path>", "error")
    return
  }
  const payload = await deps.unsetUserConfigValue(keyPath)
  appendUserConfigEffects(deps, payload)
  deps.flashFooter(`config ${keyPath} unset`, "info")
}

async function setWorkspaceLiveSyncMode(
  deps: ConfigCommandHandlerDeps,
  modeValue: string | undefined,
  rest: string[],
): Promise<void> {
  if (!deps.setUserConfigValue) {
    deps.flashFooter("user config updates are unavailable in this build", "error")
    return
  }
  const mode = modeValue ?? "required"
  if (rest.length > 0 || !["required", "unrestricted"].includes(mode)) {
    deps.flashFooter("usage: /config workspace-live-sync required|unrestricted", "error")
    return
  }
  const normalizedMode = mode === "required" ? "managed" : "unrestricted"
  const payload = await deps.setUserConfigValue("providers.workspace_live_sync", normalizedMode)
  appendUserConfigEffects(deps, payload)
  deps.flashFooter(`workspace live sync set to ${mode}`, "info")
}
