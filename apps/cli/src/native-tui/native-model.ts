import { getProviderCatalog } from "../provider-api.js"
import type { LocalIpcClient } from "../ipc.js"

/**
 * The model a native launch runs: the one asked for, or the provider's own
 * default. The kernel reads the provider's model list, so a retired name never
 * reaches the terminal as "Connection lost".
 */
export async function resolveNativeModel(
  client: LocalIpcClient,
  provider: string,
  displayName: string,
  requested: string | undefined,
  accountHint?: string,
): Promise<string> {
  const catalog = await getProviderCatalog(client, null, { provider }, false)
  const served = Object.keys(catalog.all.find((entry) => entry.id === provider)?.models ?? {})
  if (served.length === 0) {
    throw new Error(`${displayName} reports no models. Sign in to it in its own tool (for Codex: \`codex login\`), then retry.`)
  }
  if (requested === undefined) {
    const fallback = catalog.default[provider]
    if (!fallback || !served.includes(fallback)) {
      throw new Error(`${displayName} names no default model. Pass one with --model: ${served.join(", ")}.`)
    }
    return fallback
  }
  if (!served.includes(requested)) {
    if (accountHint) {
      throw new Error(`${displayName} model \`${requested}\` needs an account that is not connected. ${accountHint} Models that run without one: ${served.join(", ")}.`)
    }
    throw new Error(`${displayName} does not serve model \`${requested}\`. Pass one of these with --model, or leave --model out for the default: ${served.join(", ")}.`)
  }
  return requested
}
