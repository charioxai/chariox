import { acceptAppHostActionRequest } from "@chariox/kernel-client/ipc-requests"
import { copyTextToClipboard } from "./clipboard.js"
import { kernelApprovals } from "./kernel-approval-controller.js"
import type { RuntimeSession } from "./cli-types.js"
import { openExternalUrl } from "./external-url.js"

export type AppHostTerminal = {
  copy(text: string): Promise<void>
  openLink(url: string): Promise<boolean>
}

export function createAppHostTerminal(renderer: { copyToClipboardOSC52(text: string): boolean }): AppHostTerminal {
  return { copy: text => copyTextToClipboard(text, renderer), openLink: openExternalUrl }
}

export function appHostOperationIds(session: RuntimeSession): string[] {
  return kernelApprovals(session).flatMap(interaction => {
    const operation = interaction.kernel_operation_id?.match(/^host_action:(.+)$/)?.[1]
    return operation && interaction.id === `app_host_${operation}` ? [operation] : []
  })
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
  host?: AppHostTerminal,
): Promise<void> {
  const response = await send(acceptAppHostActionRequest(sessionId, operationId))
  const accepted = response.AppHostActionAccepted as { operation_id?: unknown; action?: { kind?: unknown; text?: unknown; url?: unknown } } | undefined
  if (accepted?.operation_id !== operationId) throw new Error("That App host request is unavailable, already answered, or expired")
  const action = accepted.action
  if (action?.kind === "clipboard_write" && typeof action.text === "string") {
    let copied = false
    try { if (host) { await host.copy(action.text); copied = true } } catch { /* the visible fallback remains available */ }
    // OSC 52 has no acknowledgement. Keep a visible fallback even after
    // the renderer/native helper succeeds.
    notice(`${copied ? "Clipboard copy requested. If your terminal cannot copy, use" : "Clipboard copy unavailable. Use"} this text: ${visibleAppClipboardText(action.text)}`)
    return
  }
  if (action?.kind === "open_link" && typeof action.url === "string") {
    // Defence at the terminal boundary as well; no shell or alternate scheme.
    const parsed = new URL(action.url)
    if (!["http:", "https:"].includes(parsed.protocol) || parsed.username || parsed.password || /[\s\u0000-\u001f\u007f\\]/u.test(action.url)) throw new Error("Invalid App link")
    let opened = false
    try { opened = await host?.openLink(action.url) ?? false } catch { /* show the exact URL below */ }
    notice(`${opened ? "Opening" : "Open this link in your browser:"} ${action.url}`)
    return
  }
  throw new Error("Invalid App host response")
}
