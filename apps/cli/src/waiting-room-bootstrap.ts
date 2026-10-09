import type { BootstrapState, CliOptions } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import type { CharioxPreferences } from "./preferences.js"
import type { BootstrapDeps } from "./session-bootstrap.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"

export function bootstrapWaitingRoom(client: LocalIpcClient, options: CliOptions, preferences: CharioxPreferences, deps: BootstrapDeps): BootstrapState {
  const useConfiguredProvider = !options.provider
  const useConfiguredModel = options.model === "default"
  const useConfiguredEffort = !options.effort.trim()
  const defaults = Promise.resolve().then(async () => {
    const configured = await deps.getConfiguredProviderLaunchDefaults?.(client) ?? {}
    return {
      ...(useConfiguredProvider && configured.provider ? { provider: configured.provider } : {}),
      ...(useConfiguredModel && configured.model ? { model: configured.model } : {}),
      ...(useConfiguredEffort && configured.effort ? { effort: configured.effort } : {}),
    }
  })
  const providerCatalog = defaults.then(() => deps.getProviderCatalog(client, deps.logger))
  const providerCommandCatalogs = Promise.resolve().then(() => deps.getProviderCommandCatalogs(client, deps.logger))
  const terminalCommandCatalog = Promise.resolve().then(() => deps.getTerminalCommandCatalog(client, deps.logger))
  const waitingRoomDefaults = defaults
  // Mount attaches consumers immediately afterward; failed network reads must
  // not become unhandled rejections during rendering initialization.
  for (const pending of [providerCatalog, providerCommandCatalogs, terminalCommandCatalog, waitingRoomDefaults]) void pending.catch(() => {})
  return {
    client, binding: null, sessions: [], options, preferences,
    providerCatalog: fallbackProviderCatalog({ source: "local_fallback" }),
    providerCommandCatalogs: fallbackProviderCommandCatalogs({ catalogSource: "local_fallback" }),
    terminalCommandCatalog: null,
    deferred: { providerCatalog, providerCommandCatalogs, terminalCommandCatalog, waitingRoomDefaults },
  }
}
