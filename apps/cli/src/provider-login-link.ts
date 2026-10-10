import { isSshTerminal } from "./clipboard.js"

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
  return [...new Set(providerLoginUrlRanges(plain).map(range => range.url))]
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


export function providerLoginUrlRanges(text: string): Array<{ url: string; start: number; end: number }> {
  return Array.from(text.matchAll(/https?:\/\/[^\s<>\u0000-\u001f\u007f]+/gu))
    .filter(match => providerLoginUrl(match[0]))
    .map(match => ({ url: match[0], start: match.index!, end: match.index! + match[0].length }))
}
