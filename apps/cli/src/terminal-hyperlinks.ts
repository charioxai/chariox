import type { CliRenderer } from "@opentui/core"
import { loadTerminalRenderLibrary } from "./terminal-render-library.js"

/** MP-08/MP-11: OpenTUI 0.1.87 renders `link()` text as OSC 8 hyperlinks only
 * after its native `setHyperlinksCapability`, which its JavaScript API never
 * calls, so long links were plain text. Terminals without OSC 8 ignore the
 * sequence. `CHARIOX_TUI_HYPERLINKS=off` keeps plain text. */
export async function enableTerminalHyperlinks(renderer: CliRenderer): Promise<boolean> {
  const rendererPtr = (renderer as { rendererPtr?: unknown } | null)?.rendererPtr
  if (process.env.CHARIOX_TUI_HYPERLINKS === "off" || !rendererPtr || !process.versions.bun) return false
  try {
    // Terminal setup initializes native capabilities; apply after it.
    await (renderer as { setupTerminal?: () => Promise<void> }).setupTerminal?.()
    const ffiModule: string = "bun:ffi"
    const { dlopen, FFIType } = await import(ffiModule)
    // The same library OpenTUI loaded, including in the standalone release.
    let libPath = await loadTerminalRenderLibrary()
    if (libPath.includes("$bunfs") || /^B:[\\/]~BUN/i.test(libPath)) libPath = libPath.replace("../", "")
    const lib = dlopen(libPath, { setHyperlinksCapability: { args: [FFIType.ptr, FFIType.bool], returns: FFIType.void } })
    // Reapply synchronously: native capability replies may reset this flag
    // immediately before a frame. Awaiting another import in the callback
    // races that frame and leaves its unchanged link cells without OSC 8.
    const apply = () => {
      lib.symbols.setHyperlinksCapability(rendererPtr, true)
      renderer.requestRender()
    }
    renderer.on("capabilities", apply)
    renderer.once("destroy", () => {
      renderer.off("capabilities", apply)
      lib.close()
    })
    apply()
    return true
  } catch {
    return false
  }
}
