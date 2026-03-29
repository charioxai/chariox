export type ProviderCatalog = {
  all: ProviderInfo[]
  default: Record<string, string>
  connected: string[]
}

export type ProviderInfo = {
  id: string
  name: string
  models: Record<string, ProviderModel>
}

export type ProviderModel = {
  id: string
  name: string
  status: string
  limit?: {
    context: number
    input?: number
    output?: number
  }
  variants?: Record<string, unknown>
}

export type CatalogModelOption = {
  id: string
  providerId: string
  providerName: string
  label: string
  variants: string[]
}

export function fallbackProviderCatalog() {
  return {
    all: [
      {
        id: "openai",
        name: "OpenAI",
        models: {
          "gpt-5.4": {
            id: "gpt-5.4",
            name: "GPT-5.4",
            status: "active",
            variants: {
              low: {},
              medium: {},
              high: {},
            },
          },
        },
      },
    ],
    default: {
      openai: "gpt-5.4",
    },
    connected: ["openai"],
  } satisfies ProviderCatalog
}

export function catalogModelOptions(catalog: ProviderCatalog) {
  const connectedProviderIds = new Set(catalog.connected)
  return catalog.all
    .filter((provider) => connectedProviderIds.has(provider.id))
    .flatMap((provider) =>
      Object.values(provider.models)
        .filter((model) => model.status !== "deprecated")
        .map((model) => ({
          id: `${provider.id}/${model.id}`,
          providerId: provider.id,
          providerName: provider.name,
          label: model.name || model.id,
          variants: Object.keys(model.variants ?? {}),
        })),
    )
    .sort((left, right) => {
      if (left.providerId === right.providerId) {
        return left.label.localeCompare(right.label)
      }
      return left.providerName.localeCompare(right.providerName)
    })
}

export function selectConfiguredModel(catalog: ProviderCatalog, configured?: string | null) {
  const options = catalogModelOptions(catalog)
  if (options.length === 0) {
    return null
  }
  const exact = configured ? options.find((option) => option.id === configured) : null
  if (exact) {
    return exact
  }
  for (const provider of catalog.all) {
    const modelId = catalog.default[provider.id]
    if (!modelId) {
      continue
    }
    const match = options.find((option) => option.id === `${provider.id}/${modelId}`)
    if (match) {
      return match
    }
  }
  return options[0] ?? null
}

export function selectConfiguredVariant(option: CatalogModelOption | null, configured?: string | null) {
  if (!option || option.variants.length === 0) {
    return ""
  }
  if (configured && option.variants.includes(configured)) {
    return configured
  }
  return option.variants.includes("high") ? "high" : option.variants[0]!
}
