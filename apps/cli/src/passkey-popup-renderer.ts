import {
  BoxRenderable, MouseButton, RGBA, ScrollBoxRenderable, TextAttributes, TextRenderable, type CliRenderer,
} from "@opentui/core"
import type { PasskeyPopupView } from "./passkey-popup-controller.js"
import { theme } from "./theme.js"
import { requesterLabels } from "./kernel-access-requester-label.js"

/** The hot-keys popup's width. */
const POPUP_WIDTH = 72

function expiry(expiresAtMs: number): string {
  return new Date(expiresAtMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })
}

/** Rows a wrapped text takes at `width`, near enough to size its box. */
function hours(minutes: number | null | undefined): string {
  const value = (minutes ?? 60) / 60
  return `${value} hour${value === 1 ? "" : "s"}`
}

function rows(text: string, width: number): number {
  return text.split("\n").reduce((count, line) => count + Math.max(1, Math.ceil(Array.from(line).length / width)), 0)
}

/** Protocol 403: the passkey popup, shaped like the hot-keys popup: a scrim
 * over the whole terminal and a centered panel. It shows only what the kernel
 * established, and the passkey as dots. Hidden, it leaves an indicator. */
export function createPasskeyPopupRenderer(renderer: CliRenderer, actions: {
  show(): void
  approve(): void
  refuse(): void
  cycleRemember(): void
}) {
  let box: BoxRenderable | undefined
  let body: ScrollBoxRenderable | undefined
  let lastFrame = ""
  const text = (content: string, options: { accent?: boolean; muted?: boolean; bold?: boolean } = {}) =>
    new TextRenderable(renderer, {
      content, wrapMode: "word", flexShrink: 0,
      fg: options.accent ? theme.primary : options.muted ? theme.textMuted : theme.text,
      attributes: options.bold ? TextAttributes.BOLD : TextAttributes.NONE,
    })
  const button = (label: string, enabled: boolean, action: () => void) => {
    const node = text(`[ ${label} ]`, { accent: enabled, muted: !enabled, bold: enabled })
    node.onMouseUp = (event) => {
      event.stopPropagation()
      if (enabled && event.button === MouseButton.LEFT) action()
    }
    return node
  }
  return {
    assign(value: BoxRenderable) { box = value; lastFrame = "" },
    scroll(direction: -1 | 1) { body?.scrollBy(direction * 4) },
    render(view: PasskeyPopupView, dimensions: { width: number; height: number }) {
      if (!box) return
      const frame = JSON.stringify([view, dimensions, theme.text, theme.primary, theme.backgroundPanel, theme.backgroundElement])
      if (lastFrame === frame) return
      lastFrame = frame
      body = undefined
      for (const child of [...box.getChildren()]) {
        box.remove(String(child.id))
        child.destroyRecursively()
      }
      box.visible = view.count > 0
      if (!view.count || !view.prompt) {
        box.requestRender()
        return
      }
      box.width = dimensions.width
      if (!view.open) {
        box.height = 2
        const indicator = new BoxRenderable(renderer, {
          position: "absolute", right: 0, top: 1, height: 1,
          backgroundColor: theme.backgroundElement, paddingLeft: 1, paddingRight: 1,
        })
        indicator.add(text(`Chariox · ${view.count} passkey request${view.count === 1 ? "" : "s"} · F8`, { accent: true }))
        indicator.onMouseUp = (event) => {
          event.stopPropagation()
          if (event.button === MouseButton.LEFT) actions.show()
        }
        box.add(indicator)
        box.requestRender()
        return
      }
      box.height = dimensions.height
      const top = Math.max(1, Math.floor(dimensions.height / 5))
      const width = Math.min(POPUP_WIDTH, dimensions.width - 4)
      const scrim = new BoxRenderable(renderer, {
        position: "absolute", left: 0, top: 0, width: dimensions.width, height: dimensions.height,
        alignItems: "center", paddingTop: top, backgroundColor: RGBA.fromInts(0, 0, 0, 150),
      })
      // Only the popup's own controls answer; a stray click does nothing.
      scrim.onMouseUp = (event) => event.stopPropagation()
      const panel = new BoxRenderable(renderer, {
        width, backgroundColor: theme.backgroundPanel,
        paddingTop: 1, paddingBottom: 1, paddingLeft: 2, paddingRight: 2, flexDirection: "column", gap: 1,
      })
      panel.onMouseUp = (event) => event.stopPropagation()
      const section = (...children: TextRenderable[]) => {
        const node = new BoxRenderable(renderer, { flexDirection: "column", flexShrink: 0 })
        for (const child of children) node.add(child)
        panel.add(node)
        return node
      }
      const header = new BoxRenderable(renderer, { flexDirection: "row", justifyContent: "space-between", flexShrink: 0 })
      header.add(text("Chariox passkey", { bold: true }))
      header.add(text("Esc hides", { muted: true }))
      panel.add(header)
      const prompt = view.prompt
      // The request scrolls when the terminal is too short for all of it.
      const fixedRows = 14 + (view.error ? 2 : 0)
      const requester = prompt.kind === "access_grant" || prompt.kind === "access_extension"
        ? requesterLabels(prompt.requester) : []
      const wanted = requester.reduce((count, label) => count + rows(label, width - 5), 0) + 2 + rows(prompt.title || "Critical approval", width - 5) + rows(prompt.message, width - 5)
      body = new ScrollBoxRenderable(renderer, {
        height: Math.max(2, Math.min(wanted, dimensions.height - top - 1 - fixedRows)),
        flexShrink: 0, scrollY: true, scrollX: false,
      })
      body.add(text(`${prompt.kind === "critical_approval" ? "Critical approval" : prompt.kind === "sudo" ? prompt.lifetime_minutes ? "Sudo window" : "Sudo operation scope" : "External agent access"}${view.count > 1 ? ` · ${view.index + 1} of ${view.count}` : ""}`, { muted: true }))
      body.add(text(prompt.title || "Critical approval", { accent: true, bold: true }))
      for (const label of requester) body.add(text(label))
      body.add(text(prompt.message))
      panel.add(body)
      const session = prompt.session_alias ? `${prompt.session_alias} (${prompt.session_id})` : prompt.session_id
      section(text(`${prompt.session_id === "kernel-access" && (prompt.kind === "access_grant" || prompt.kind === "access_extension")
        ? "Local kernel" : `Session: ${session}`} · expires ${expiry(prompt.expires_at_ms)}`, { muted: true }))
      // Hidden input: only the length is ever rendered.
      const remember = text(prompt.kind === "critical_approval"
        ? `Remember for: ${view.passkey.rememberMinutes ? `${view.passkey.rememberMinutes} minutes` : "off"}`
        : prompt.kind === "sudo" ? prompt.lifetime_minutes ? `Window: ${hours(view.passkey.accessLifetimeMinutes)} · maximum ${hours(prompt.max_lifetime_minutes)} · fresh passkey required` : "Operation scope · fresh passkey required"
        : `Access for: ${view.passkey.accessLifetimeMinutes} minutes · maximum ${prompt.max_lifetime_minutes}`)
      remember.onMouseUp = (event) => {
        event.stopPropagation()
        if (event.button === MouseButton.LEFT) actions.cycleRemember()
      }
      const input = text(`Passkey: ${"•".repeat(Math.min(view.passkey.length, 40))}▏`, { accent: true })
      section(input, remember)
      if (view.error) section(text(view.error))
      const enabled = view.connected && !view.pending
      const buttons = new BoxRenderable(renderer, { flexDirection: "row", gap: 2, flexShrink: 0 })
      buttons.add(button("Approve", enabled, actions.approve))
      buttons.add(button("Refuse", enabled, actions.refuse))
      panel.add(buttons)
      section(text(view.pending ? "Waiting for the kernel…"
        : !view.connected ? "Disconnected · reconnect to answer"
        : `Enter approves · Ctrl+R refuses${view.prompt?.kind === "critical_approval" ? " · Tab remember" : view.prompt?.kind === "sudo" ? view.prompt.lifetime_minutes ? " · Tab duration" : "" : " · Tab lifetime"}${view.count > 1 ? " · ←/→ requests" : ""}`, { muted: true }))
      scrim.add(panel)
      box.add(scrim)
      box.requestRender()
    },
  }
}
