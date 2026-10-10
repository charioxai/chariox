import { openExternalUrl } from "./external-url.js"
import { localDesktopAvailable, providerLoginUrl, providerLoginUrlRanges } from "./provider-login-link.js"

declare const Bun: { stringWidth(text: string): number }

type LinkMouseEvent = { button: number; x: number; y: number; target: unknown }
type LinkPress = { x: number; y: number; target: unknown; selection: unknown }

type TextTarget = { plainText: string; getSelection(): { start: number; end: number } | null }

export function createTranscriptLinkController(deps: {
  renderer: { getSelection(): unknown; updateSelection?(target: any, x: number, y: number): void }
  flashFooter(message: string, tone: "info" | "error"): void
  localDesktop?: () => boolean
  openUrl?: typeof openExternalUrl
}) {
  let pressed: LinkPress | null = null
  let clicked: { url: string; selection: unknown } | null = null
  const activate = async (url: string): Promise<boolean> => {
    if (!providerLoginUrl(url)) return false
    if ((deps.localDesktop ?? localDesktopAvailable)()) {
      const opened = await (deps.openUrl ?? openExternalUrl)(url)
      deps.flashFooter(opened ? "browser open requested" : "could not open browser; F6 copies the URL, F7 shows it", opened ? "info" : "error")
      return opened
    }
    // SSH hyperlinks are OSC 8; the user's terminal opens them. With no
    // hyperlink support, F6 copies the full clicked URL through the normal path.
    deps.flashFooter("Cmd-click opens link; F6 copies URL; F7 shows clean text", "info")
    return false
  }
  return {
    activate,
    selectedUrl: () => clicked && deps.renderer.getSelection() === clicked.selection ? clicked.url : null,
    handleMouseDown(event: LinkMouseEvent) {
      pressed = null
      clicked = null
      if (event.button !== 0 || !event.target) return
      const target = event.target as Partial<TextTarget>
      if (typeof target.plainText !== "string" || typeof target.getSelection !== "function") return
      pressed = { x: event.x, y: event.y, target: event.target, selection: deps.renderer.getSelection() }
    },
    handleMouseDrag() { pressed = null },
    handleMouseUp(event: LinkMouseEvent) {
      const down = pressed
      pressed = null
      if (!down || event.button !== 0 || down.target !== event.target || down.x !== event.x || down.y !== event.y) return
      const target = down.target as TextTarget
      // A point selection has no text range. Ask the same renderer to extend
      // it by one cell for the logical offset; restore a non-link immediately.
      if (!target.getSelection()) deps.renderer.updateSelection?.(down.target, event.x + 1, event.y)
      const offset = target.getSelection()?.start
      const range = offset === undefined ? null : providerLoginUrlRanges(target.plainText)
        .find(range => {
          // TextBuffer selections use logical display columns (wide glyphs,
          // default two-column tabs, and one slot for each logical newline).
          const before = target.plainText.slice(0, range.start).replace(/\n/g, " ").replace(/\t/g, "  ")
          const start = Bun.stringWidth(before)
          return offset >= start && offset < start + Bun.stringWidth(range.url)
        })
      if (!range) { deps.renderer.updateSelection?.(down.target, event.x, event.y); return }
      clicked = { selection: down.selection, url: range.url }
      void activate(range.url).catch(() => deps.flashFooter("could not open link; F6 copies the URL, F7 shows it", "error"))
    },
  }
}
