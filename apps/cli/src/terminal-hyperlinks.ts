import type { CliRenderer } from "@opentui/core"
import { loadTerminalRenderLibrary } from "./terminal-render-library.js"

/** MP-08/MP-11: OpenTUI 0.1.87 renders `link()` text as OSC 8 hyperlinks only
 * after its native `setHyperlinksCapability`, which its JavaScript API never
 * calls, so long links were plain text. Terminals without OSC 8 ignore the
 * sequence. `CHARIOX_TUI_HYPERLINKS=off` keeps plain text. */
export async function enableTerminalHyperlinks(renderer: CliRenderer): Promise<boolean> {
  const rendererPtr = (renderer as { rendererPtr?: unknown } | null)?.rendererPtr
  if (process.env.CHARIOX_TUI_HYPERLINKS === "off" || !rendererPtr || !process.versions.bun) return false
  let destroyed = renderer.root.isDestroyed
  const markDestroyed = () => { destroyed = true }
  renderer.once("destroy", markDestroyed)
  try {
    // Terminal setup initializes native capabilities; apply after it.
    await (renderer as { setupTerminal?: () => Promise<void> }).setupTerminal?.()
    const ffiModule: string = "bun:ffi"
    const { dlopen, FFIType } = await import(ffiModule)
    // The same library OpenTUI loaded, including in the standalone release.
    let libPath = await loadTerminalRenderLibrary()
    if (libPath.includes("$bunfs") || /^B:[\\/]~BUN/i.test(libPath)) libPath = libPath.replace("../", "")
    const lib = dlopen(libPath, { setHyperlinksCapability: { args: [FFIType.ptr, FFIType.bool], returns: FFIType.void } })
    if (destroyed) { lib.close(); return false }
    renderer.off("destroy", markDestroyed)
    const native = renderer as unknown as { renderNative(): void }
    const original = native.renderNative
    const render = function (this: typeof native) {
      // Reinitialization can reset native capabilities without a JS event.
      lib.symbols.setHyperlinksCapability(rendererPtr, true)
      original.call(this)
    }
    native.renderNative = render
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
      if (native.renderNative === render) native.renderNative = original
      lib.close()
    })
    apply()
    return true
  } catch {
    renderer.off("destroy", markDestroyed)
    return false
  }
}
