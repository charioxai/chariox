import type { EventEmitter } from "node:events"
import { writeSync } from "node:fs"
import { stripVTControlCharacters } from "node:util"
import { providerLoginLinkText, providerLoginUrl } from "./provider-login-link.js"

/** MP-08 / MP-10: logical text, without panel padding or cursor cells.
 * MP-11: selected provider text must never become terminal instructions. */
export function selectionTextViewContent(text: string): string {
  return stripVTControlCharacters(text).replace(/\r\n?/g, "\n")
    .replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/g, "")
    .replace(/\n/g, "\r\n")
}

type SelectionTerminal = {
  input: Pick<EventEmitter, "on" | "once" | "removeListener"> & {
    isTTY?: boolean; isRaw?: boolean; setRawMode(mode: boolean): unknown; resume(): unknown
  }
  output: { isTTY?: boolean; fd: number }
}

export function createSelectionTextView(
  renderer: { suspend(): void; resume(): void; idle(): Promise<void>; clearSelection(): void; isDestroyed?: boolean },
  onError: (error: unknown) => void,
  terminal: SelectionTerminal = { input: process.stdin, output: process.stdout },
) {
  let active = false
  let cancelled = false
  let finish: (() => void) | undefined
  const present = (text: string): boolean => {
    const { input, output } = terminal
    const content = selectionTextViewContent(text)
    if (active || !content || !input.isTTY || !output.isTTY) return false
    active = true
    cancelled = false
    const wasRaw = input.isRaw ?? false
    const show = async () => {
      try {
        renderer.suspend()
        await renderer.idle()
        if (cancelled) return
        input.setRawMode(true)
        input.resume()
        // Normal terminal buffer: the terminal soft-wraps each logical line.
        // Keep all instructions before the text and no layout beside it.
        writeSync(output.fd, "\x1b[?1049l\x1b[0m\r\n\x1b[JSelected text — drag-select, Cmd-C; Enter/Esc/F7 returns.\r\n\r\n")
        const url = providerLoginUrl(content)
        writeSync(output.fd, url ? providerLoginLinkText(url) : content + "\r\n")
        await new Promise<void>(resolve => {
          const onData = (chunk: Buffer | string) => {
            if (/^(?:[\r\n\x03\x1b]|\x1b\[18~)$/.test(chunk.toString())) finish?.()
          }
          finish = () => {
            input.removeListener("data", onData)
            input.removeListener("end", finish!)
            finish = undefined
            resolve()
          }
          input.on("data", onData)
          input.once("end", finish)
        })
      } finally {
        input.setRawMode(wasRaw)
        active = false
        if (!renderer.isDestroyed) { renderer.clearSelection(); renderer.resume() }
      }
    }
    void show().catch(onError)
    return true
  }
  return { present, isActive: () => active, cancel: () => { cancelled = true; finish?.() } }
}
