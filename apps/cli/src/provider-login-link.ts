import { writeSync } from "node:fs"
import { clipboardCopyMessage, copyTextToClipboard } from "./clipboard.js"
import { openExternalUrl } from "./external-url.js"
import { isSshTerminal } from "./terminal-copy.js"

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

export function providerLoginLinkText(url: string): string {
  if (!providerLoginUrl(url)) throw new Error("Invalid provider authorization URL")
  // One logical line: the terminal alone soft-wraps it, without layout padding.
  return `\x1b]8;;${url}\x1b\\${url}\x1b]8;;\x1b\\\r\n`
}

type LoginLinkRenderer = {
  suspend(): void
  resume(): void
  idle(): Promise<void>
}

/** Hand off to the normal terminal buffer, preserving the link in scrollback.
 * Mouse reporting is suspended, so native selection works without a modifier. */
export function createProviderLoginLinkPresenter(renderer: LoginLinkRenderer) {
  let active = false
  return async (url: string, userCode?: string | null): Promise<boolean> => {
    if (!providerLoginUrl(url) || !process.stdin.isTTY || !process.stdout.isTTY || active) return false
    active = true
    const write = (text: string) => { writeSync(process.stdout.fd, text) }
    try {
      renderer.suspend()
      // Drain an in-flight OpenTUI frame before writing outside its layout.
      await renderer.idle()
      process.stdin.setRawMode(true)
      process.stdin.resume()
      write("\x1b[?1049l\x1b[0m\r\n\x1b[JProvider authorization link (Cmd-click if supported):\r\n")
      write(providerLoginLinkText(url))
      if (userCode && /^[A-Za-z0-9 -]{1,128}$/.test(userCode)) write(`Device code: ${userCode}\r\n`)
      write("C copies the full URL; O opens a local browser; Enter returns to Chariox.\r\nNative selection + terminal Copy also works here.\r\n")
      if (localDesktopAvailable()) {
        write(await openExternalUrl(url) ? "Browser open requested.\r\n" : "Could not open the browser; use the link above.\r\n")
      } else {
        write("Open this link on your desktop (SSH/headless terminal).\r\n")
      }
      await new Promise<void>((resolve, reject) => {
        let busy = false
        const finish = () => {
          process.stdin.removeListener("data", onData)
          process.stdin.removeListener("end", finish)
          resolve()
        }
        const onData = (chunk: Buffer) => {
          const key = chunk.toString("utf8")
          if (/^[\r\n\x03\x1b]$/.test(key)) { if (!busy) finish(); return }
          if (busy || !/^[co]$/i.test(key)) return
          busy = true
          const action = key.toLowerCase() === "c"
            ? copyTextToClipboard(url, { copyToClipboardOSC52: () => false })
              .then(result => write(`${clipboardCopyMessage(result)}\r\n`))
            : (localDesktopAvailable()
              ? openExternalUrl(url).then(opened => write(opened ? "Browser open requested.\r\n" : "Could not open the browser; use the link above.\r\n"))
              : Promise.resolve(write("SSH/headless terminal: open the link on your desktop.\r\n")))
          void action.catch(error => {
            process.stdin.removeListener("data", onData)
            process.stdin.removeListener("end", finish)
            reject(error)
          }).finally(() => { busy = false })
        }
        process.stdin.on("data", onData)
        process.stdin.once("end", finish)
      })
      return true
    } finally {
      process.stdin.setRawMode(false)
      active = false
      renderer.resume()
    }
  }
}
