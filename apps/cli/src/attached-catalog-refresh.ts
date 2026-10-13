type AttachmentBinding = { sessionId: string; attachmentId: string }

// MP-08 / MP-10: catalogs enrich an attached view without delaying its input readiness.
export async function refreshAttachedCatalogs<ProviderCatalog, CommandCatalog>(
  binding: AttachmentBinding,
  deps: {
    isCurrent: () => boolean
    getProviderCatalog: () => Promise<ProviderCatalog>
    setProviderCatalog: (catalog: ProviderCatalog) => void
    getCommandCatalog: () => Promise<CommandCatalog>
    setCommandCatalog: (catalog: CommandCatalog) => void
    formatError: (error: unknown) => string
    logWarning?: (message: string, fields: Record<string, unknown>) => void
  },
): Promise<void> {
  const refresh = async <Catalog>(
    label: string,
    load: () => Promise<Catalog>,
    apply: (catalog: Catalog) => void,
  ) => {
    try {
      const catalog = await load()
      if (deps.isCurrent()) apply(catalog)
    } catch (error) {
      if (deps.isCurrent()) {
        deps.logWarning?.(`failed to refresh ${label} after attach`, {
          session_id: binding.sessionId,
          attachment_id: binding.attachmentId,
          error: deps.formatError(error),
        })
      }
    }
  }
  await Promise.allSettled([
    refresh("provider catalog", deps.getProviderCatalog, deps.setProviderCatalog),
    refresh("terminal command catalog", deps.getCommandCatalog, deps.setCommandCatalog),
  ])
}
