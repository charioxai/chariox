import { looksLikeDrillSecretValue } from "./drill-secrets.mjs"

export const DRILL_PROVIDER_IDS = Object.freeze([
  "claude",
  "claude-headless",
  "claude-p",
  "codex",
  "dev-stub",
  "opencode",
  "opencode-zen",
])

export function isKnownDrillProvider(provider) {
  return DRILL_PROVIDER_IDS.includes(provider)
}

export function parseProviderList(value) {
  const providers = String(value ?? "")
    .split(",")
    .map((provider) => provider.trim())
    .filter(Boolean)
  if (providers.length === 0) throw new Error("provider list must contain at least one provider")
  validateDrillProviders(providers, "provider list")
  return providers
}

export function parseProviderAccountAlias(value) {
  const [provider, alias] = String(value ?? "").split("=", 2).map((part) => part.trim())
  if (!provider || !alias) throw new Error("--provider-account must use provider=alias")
  validateProviderAccountAlias(alias)
  return { provider, alias }
}

export function applyProviderAccountAlias(providerAccounts, value) {
  const { provider, alias } = parseProviderAccountAlias(value)
  providerAccounts[provider] = alias
  return providerAccounts
}

export function parseProviderModelOverride(value) {
  const [provider, model] = String(value ?? "").split("=", 2).map((part) => part.trim())
  if (!provider || !model) throw new Error("--provider-model must use provider=model")
  return { provider, model }
}

export function applyProviderModelOverride(providerModels, value) {
  const { provider, model } = parseProviderModelOverride(value)
  providerModels[provider] = model
  return providerModels
}

export function resolveProviderModel(provider, {
  defaultModel,
  providerModels = {},
} = {}) {
  const explicit = providerModels[provider]
  if (explicit) return explicit
  if (provider === "opencode" && !String(defaultModel).includes("/")) return `opencode/${defaultModel}`
  if (provider === "codex" && !String(defaultModel).includes("/")) return codexCliModel(defaultModel)
  return defaultModel
}

export function codexCliModel(model) {
  if (String(model).endsWith("-codex")) return model
  if (/^gpt-5\.[23]$/.test(String(model))) return `${model}-codex`
  return model
}

export function providerProfileMetadata({ providers, defaultModel, providerModels = {}, providerAccounts = {} }) {
  const normalizedProviders = [...new Set((providers ?? []).filter(Boolean))].sort()
  validateDrillProviders(normalizedProviders, "provider profile providers")
  const providerSet = new Set(normalizedProviders)
  const accountProviders = Object.keys(providerAccounts).sort()
  for (const provider of accountProviders) {
    validateKnownDrillProvider(provider, "provider account alias provider")
    if (!providerSet.has(provider)) {
      throw new Error(`provider account alias references unselected provider: ${provider}`)
    }
    validateProviderAccountAlias(providerAccounts[provider])
  }
  for (const provider of Object.keys(providerModels)) {
    validateKnownDrillProvider(provider, "provider model override provider")
    if (!providerSet.has(provider)) {
      throw new Error(`provider model override references unselected provider: ${provider}`)
    }
  }
  return {
    providerCount: normalizedProviders.length,
    providers: normalizedProviders.join(","),
    defaultModel,
    providerModelOverrides: Object.keys(providerModels).sort().join(","),
    providerAccountAliases: accountProviders.map((provider) => `${provider}=${providerAccounts[provider]}`).join(","),
  }
}

export function validateDrillProviders(providers, source) {
  if (!Array.isArray(providers) || !providers.every((provider) => typeof provider === "string" && provider.length > 0)) {
    throw new Error(`${source} has invalid providers`)
  }
  for (const [index, provider] of providers.entries()) {
    validateKnownDrillProvider(provider, `${source}[${index}]`)
  }
}

function validateKnownDrillProvider(provider, source) {
  if (!isKnownDrillProvider(provider)) {
    throw new Error(`${source} has unknown provider ${JSON.stringify(provider)}`)
  }
}

function validateProviderAccountAlias(alias) {
  if (typeof alias !== "string" || !/^[a-zA-Z0-9._-]{1,64}$/.test(alias) || looksLikeDrillSecretValue(alias)) {
    throw new Error("provider account alias must be a non-secret label using letters, numbers, dots, dashes, or underscores")
  }
}
