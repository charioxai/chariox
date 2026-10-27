// MP-08 / MP-10 / MP-11: reconcile changed inventories below composer wiring.
import type { ComposerInventory } from "./cli-types.js"
type Catalogs = Pick<ComposerInventory, "providerCatalog" | "providerCommandCatalogs" | "terminalCommandCatalog" | "catalogRevisions">
export async function reconcileComposerCatalogs(cached: Catalogs | undefined, reads: {
  revisions(): Promise<NonNullable<Catalogs["catalogRevisions"]> | null | undefined>
  provider(): Promise<Catalogs["providerCatalog"]>
  providerCommands(): Promise<Catalogs["providerCommandCatalogs"]>
  terminalCommands(): Promise<Catalogs["terminalCommandCatalog"]>
}): Promise<Catalogs> {
  const revisions = await reads.revisions()
  const unchanged = (key: "provider_catalog" | "provider_commands" | "terminal_commands") => revisions && cached?.catalogRevisions?.[key] === revisions[key]
  const [providerCatalog, providerCommandCatalogs, terminalCommandCatalog] = await Promise.all([
    unchanged("provider_catalog") ? cached!.providerCatalog : reads.provider(),
    unchanged("provider_commands") ? cached!.providerCommandCatalogs : reads.providerCommands(),
    unchanged("terminal_commands") ? cached!.terminalCommandCatalog : reads.terminalCommands(),
  ])
  return { providerCatalog, providerCommandCatalogs, terminalCommandCatalog, ...(revisions ? { catalogRevisions: revisions } : {}) }
}
