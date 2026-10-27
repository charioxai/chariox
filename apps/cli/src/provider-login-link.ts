import { isSshTerminal } from "./clipboard.js"
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

export type ProviderLoginLinkOptions = { userCode?: string | null; autoOpen?: boolean }

/** MP-08/MP-10/MP-11: links stay in the transcript. Only an explicit
 * local login start can ask the desktop browser to open; SSH uses the
 * terminal's native hyperlinks and never switches terminal buffers. */
export function createProviderLoginLinkOpener(deps = {
  localDesktop: localDesktopAvailable,
  openUrl: openExternalUrl,
}) {
  const opened = new Set<string>()
  return async (url: string, options: ProviderLoginLinkOptions = {}): Promise<boolean> => {
    if (!providerLoginUrl(url) || !options.autoOpen || !deps.localDesktop() || opened.has(url)) return false
    const success = await deps.openUrl(url)
    if (success) opened.add(url)
    return success
  }
}
