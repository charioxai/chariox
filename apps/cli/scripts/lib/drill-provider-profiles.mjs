export function parseProviderList(value) {
  const providers = String(value ?? "")
    .split(",")
    .map((provider) => provider.trim())
    .filter(Boolean)
  if (providers.length === 0) throw new Error("provider list must contain at least one provider")
  return providers
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

export function providerProfileMetadata({ providers, defaultModel, providerModels = {} }) {
  return {
    providerCount: providers.length,
    providers: providers.join(","),
    defaultModel,
    providerModelOverrides: Object.keys(providerModels).sort().join(","),
  }
}
