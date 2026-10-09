import type { EventEmitter } from "node:events"
import { writeSync } from "node:fs"
import { clipboardCopyMessage, copyTextToClipboard, isSshTerminal } from "./clipboard.js"
import { openExternalUrl } from "./external-url.js"

/** Only complete web URLs, never terminal controls or alternate URL schemes. */
export function providerLoginUrl(value: string): string | null {
  if (/[\s\u0000-\u001f\u007f\\]/u.test(value)) return null
  try {
    const url = new URL(value)
    return ["https:", "http:"].includes(url.protocol) && !url.username && !url.password ? value : null
  } catch { return null }
}

export function providerLoginUrls(output: string): string[] {
  // Official provider PTYs may decorate an otherwise unbroken URL with ANSI.
  const plain = output.replace(/\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, "")
  return [...new Set((plain.match(/https?:\/\/[^\s<>\u0000-\u001f\u007f]+/gu) ?? [])
    .filter(value => providerLoginUrl(value)))]
}

export function localDesktopAvailable(): boolean {
  return !isSshTerminal() && (process.platform === "darwin" || process.platform === "win32"
    || Boolean(process.env.DISPLAY || process.env.WAYLAND_DISPLAY))
}

function oneLine(text: string): string {
  return text.replace(/[\u0000-\u001f\u007f]/g, " ")
}

export function providerLoginLinkText(url: string): string {
  if (!providerLoginUrl(url)) throw new Error("Invalid provider authorization URL")
  // One logical line: the terminal alone soft-wraps it, without layout padding.
  return `\x1b]8;;${url}\x1b\\${url}\x1b]8;;\x1b\\\r\n`
}

type LoginLinkRenderer = Parameters<typeof copyTextToClipboard>[1] & {
  suspend(): void
  resume(): void
  idle(): Promise<void>
}

type LoginLinkTerminal = {
  input: Pick<EventEmitter, "on" | "once" | "removeListener"> & { isTTY?: boolean; setRawMode(mode: boolean): unknown; resume(): unknown }
  output: { isTTY?: boolean; fd: number }
}

export type ProviderLoginLinkOptions = {
  userCode?: string | null
  autoOpen?: boolean
  /** Show the link again even if this terminal already showed it. */
  force?: boolean
  title?: string
  /** Steps after "Open this link"; the view numbers them. */
  steps?: string[]
  /** A bracketed paste returns to Chariox and hands over the pasted text. */
  onPaste?: (text: string) => void
}

/** Hand off to the normal terminal buffer, preserving the link in scrollback.
 * Mouse reporting is suspended, so native selection works without a modifier.
 * Each link is shown once; the transcript keeps it for later. */
export function createProviderLoginLinkPresenter(
  renderer: LoginLinkRenderer,
  terminal: LoginLinkTerminal = { input: process.stdin, output: process.stdout },
) {
  let active = false
  const shown = new Set<string>()
  const present = async (url: string, options: ProviderLoginLinkOptions = {}): Promise<boolean> => {
    const { input, output } = terminal
    if (!providerLoginUrl(url) || (shown.has(url) && !options.force) || !input.isTTY || !output.isTTY || active) return false
    active = true
    shown.add(url)
    const userCode = options.userCode
    const write = (text: string) => { writeSync(output.fd, text) }
    try {
      renderer.suspend()
      // Drain an in-flight OpenTUI frame before writing outside its layout.
      await renderer.idle()
      input.setRawMode(true)
      input.resume()
      write(`\x1b[?1049l\x1b[0m\r\n\x1b[J${oneLine(options.title ?? "Provider authorization link")}\r\n`)
      write("1. Open this link (Cmd-click it, or select and copy it):\r\n")
      write(providerLoginLinkText(url))
      if (userCode && /^[A-Za-z0-9 -]{1,128}$/.test(userCode)) write(`Device code: ${userCode}\r\n`)
      ;(options.steps ?? []).forEach((step, index) => write(`${index + 2}. ${oneLine(step)}\r\n`))
      // Bracketed paste only: an unmarked chunk could be a split paste.
      if (options.onPaste) write("\x1b[?2004h")
      write("C copies the link · O opens it on this computer · Enter returns to Chariox\r\n")
      if (!localDesktopAvailable()) {
        write("This is an SSH/headless terminal: open the link on your own computer.\r\n")
      } else if (options.autoOpen) {
        write(await openExternalUrl(url) ? "Browser open requested.\r\n" : "Could not open the browser; use the link above.\r\n")
      }
      await new Promise<void>((resolve, reject) => {
        let busy = false
        let paste: string | null = null
        const finish = () => {
          input.removeListener("data", onData)
          input.removeListener("end", finish)
          resolve()
        }
        const onData = (chunk: Buffer) => {
          const key = chunk.toString("utf8")
          if (options.onPaste && (paste !== null || key.startsWith("\x1b[200~"))) {
            paste = (paste ?? "") + key
            const end = paste.indexOf("\x1b[201~")
            if (end < 0) return
            const text = paste.slice("\x1b[200~".length, end)
            paste = null
            options.onPaste(text)
            finish()
            return
          }
          if (/^[\r\n\x03\x1b]$/.test(key)) { if (!busy) finish(); return }
          if (busy || !/^[co]$/i.test(key)) return
          busy = true
          const action = key.toLowerCase() === "c"
            ? copyTextToClipboard(url, renderer)
              .then(result => write(`${clipboardCopyMessage(result)}\r\n`))
            : (localDesktopAvailable()
              ? openExternalUrl(url).then(opened => write(opened ? "Browser open requested.\r\n" : "Could not open the browser; use the link above.\r\n"))
              : Promise.resolve(write("SSH/headless terminal: open the link on your desktop.\r\n")))
          void action.catch(error => {
            input.removeListener("data", onData)
            input.removeListener("end", finish)
            reject(error)
          }).finally(() => { busy = false })
        }
        input.on("data", onData)
        input.once("end", finish)
      })
      return true
    } finally {
      input.setRawMode(false)
      active = false
      renderer.resume()
    }
  }
  return Object.assign(present, { isActive: () => active })
}
