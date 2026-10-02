import process from "node:process"
import { acceptAppHostActionRequest } from "@chariox/kernel-client/ipc-requests"
import { openExternalUrl } from "./external-url.js"

export type AppHostTerminal = {
  copyOSC52(text: string): boolean
  openLink(url: string): Promise<boolean>
}

const terminal: AppHostTerminal = {
  copyOSC52(text) {
    if (!process.stdout.isTTY) return false
    process.stdout.write(`\x1b]52;c;${Buffer.from(text, "utf8").toString("base64")}\x07`)
    return true
  },
  openLink: openExternalUrl,
}

/** Render a complete, reversible representation without terminal controls or
 * hidden Unicode formatting. OSC 52 receives the original text unchanged. */
export function visibleAppClipboardText(text: string): string {
  return JSON.stringify(text).replace(/[\u00ad\u061c\u180e\u200b-\u200f\u202a-\u202e\u2060-\u206f\ufeff\ufff9-\ufffb]/g,
    c => `\\u${c.charCodeAt(0).toString(16).padStart(4, "0")}`)
}

export async function acceptAppHostOffer(
  sessionId: string,
  operationId: string,
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>,
  notice: (message: string) => void,
  host: AppHostTerminal = terminal,
): Promise<void> {
  const response = await send(acceptAppHostActionRequest(sessionId, operationId))
  const accepted = response.AppHostActionAccepted as { operation_id?: unknown; action?: { kind?: unknown; text?: unknown; url?: unknown } } | undefined
  if (accepted?.operation_id !== operationId) throw new Error("That App host request is unavailable, already answered, or expired")
  const action = accepted.action
  if (action?.kind === "clipboard_write" && typeof action.text === "string") {
    let copied = false
    try { copied = host.copyOSC52(action.text) } catch { /* the visible fallback remains available */ }
    // OSC 52 has no acknowledgement, so even when emitted the fallback is
    // always visible. A remote terminal controls its own clipboard support.
    notice(`${copied ? "Clipboard copy requested via OSC 52. If your terminal cannot copy, use" : "Clipboard copy unavailable. Use"} this text: ${visibleAppClipboardText(action.text)}`)
    return
  }
  if (action?.kind === "open_link" && typeof action.url === "string") {
    // Defence at the terminal boundary as well; no shell or alternate scheme.
    const parsed = new URL(action.url)
    if (!["http:", "https:"].includes(parsed.protocol) || /[\s\u0000-\u001f\u007f\\]/u.test(action.url)) throw new Error("Invalid App link")
    let opened = false
    try { opened = await host.openLink(action.url) } catch { /* show the exact URL below */ }
    notice(`${opened ? "Opening" : "Open this link in your browser:"} ${action.url}`)
    return
  }
  throw new Error("Invalid App host response")
}
