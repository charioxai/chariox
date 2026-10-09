/** MP-08/MP-11: OpenTUI 0.1.87 renders `link()` text as OSC 8 hyperlinks only
 * after its native `setHyperlinksCapability`, which its JavaScript API never
 * calls, so long links were plain text. Terminals without OSC 8 ignore the
 * sequence. `CHARIOX_TUI_HYPERLINKS=off` keeps plain text. */
export async function enableTerminalHyperlinks(renderer: unknown): Promise<boolean> {
  const rendererPtr = (renderer as { rendererPtr?: unknown } | null)?.rendererPtr
  if (process.env.CHARIOX_TUI_HYPERLINKS === "off" || !rendererPtr || !process.versions.bun) return false
  try {
    const ffiModule: string = "bun:ffi"
    const { dlopen, FFIType } = await import(ffiModule)
    // The same library OpenTUI loaded, resolved from OpenTUI's own package.
    const bun = (globalThis as unknown as { Bun: { resolveSync(specifier: string, from: string): string } }).Bun
    const core = bun.resolveSync("@opentui/core", import.meta.dirname)
    let libPath: string = (await import(bun.resolveSync(`@opentui/core-${process.platform}-${process.arch}/index.ts`, core.replace(/[\\/][^\\/]*$/, "")))).default
    if (libPath.includes("$bunfs") || /^B:[\\/]~BUN/i.test(libPath)) libPath = libPath.replace("../", "")
    const lib = dlopen(libPath, { setHyperlinksCapability: { args: [FFIType.ptr, FFIType.bool], returns: FFIType.void } })
    lib.symbols.setHyperlinksCapability(rendererPtr, true)
    return true
  } catch {
    return false
  }
}
