/** MP-08/MP-11: OpenTUI's renderer library in a source install. The release compiler binds
 * this loader to the same embedded native package as OpenTUI itself. */
export async function loadTerminalRenderLibrary(): Promise<string> {
  const bun = (globalThis as unknown as { Bun: { resolveSync(specifier: string, from: string): string } }).Bun
  const core = bun.resolveSync("@opentui/core", import.meta.dirname)
  return (await import(bun.resolveSync(`@opentui/core-${process.platform}-${process.arch}/index.ts`, core.replace(/[\\/][^\\/]*$/, "")))).default
}
