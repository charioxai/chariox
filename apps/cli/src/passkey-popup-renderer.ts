import {
  BoxRenderable, MouseButton, RGBA, ScrollBoxRenderable, TextAttributes, TextRenderable, type CliRenderer,
} from "@opentui/core"
import { UNTIL_AGENT_EXITS, type PasskeyPopupView } from "./passkey-popup-controller.js"
import { approvalCountdown } from "./kernel-clock.js"
import { theme } from "./theme.js"

/** The hot-keys popup's width. */
const POPUP_WIDTH = 72

function expiry(expiresAtMs: number): string {
  return new Date(expiresAtMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })
}

/** Rows a wrapped text takes at `width`, near enough to size its box. */
function hours(minutes: PasskeyPopupView["passkey"]["lifetime"]): string {
  const value = (typeof minutes === "number" ? minutes : 60) / 60
  return `${value} hour${value === 1 ? "" : "s"}`
}

function rows(text: string, width: number): number {
  return text.split("\n").reduce((count, line) => count + Math.max(1, Math.ceil(Array.from(line).length / width)), 0)
}

function passkeyNoticeLabel(prompt: { readonly kind: string; readonly title: string; readonly requester?: { readonly provider_harness?: string | null } | null; readonly sudo_request?: {readonly requester_name: string} | null }): string {
  const harness = prompt.requester?.provider_harness
  const who = harness === "claude" ? "Claude Code" : harness === "codex" ? "Codex" : harness === "opencode" ? "OpenCode" : "An external agent"
  if (prompt.kind === "access_grant") return `${who} asks for access`
  if (prompt.kind === "access_extension") return `${who} asks to extend access`
  if (prompt.kind === "sudo") return prompt.sudo_request?.requester_name === "You" ? "You requested one sudo task" : `${prompt.sudo_request?.requester_name ?? who} asks to run one sudo task`
  return (prompt.title || "Critical approval").replace(/[\x00-\x1f\x7f]/g, " ").slice(0,120)
}

/** Protocol 403: the passkey popup, shaped like the hot-keys popup: a scrim
 * over the whole terminal and a centered panel. It shows only what the kernel
 * established, and the passkey as dots. Hidden, it leaves an indicator. */
export function createPasskeyPopupRenderer(renderer: CliRenderer, actions: {
  show(): void
  approve(): void
  refuse(): void
  cycleRemember(): void
  toggleDetails?(): void
}) {
  let box: BoxRenderable | undefined
  let banner: BoxRenderable | undefined
  let body: ScrollBoxRenderable | undefined
  let countdownText: TextRenderable | undefined
  let noticeText: TextRenderable | undefined
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
    assignBanner(value: BoxRenderable) { banner = value; lastFrame = "" },
    scroll(direction: -1 | 1) { body?.scrollBy(direction * 4) },
    render(view: PasskeyPopupView, dimensions: { width: number; height: number }) {
      if (!box) return
      const countdown = approvalCountdown(view.remainingMs)
      const notice = view.prompt ? `Chariox · ${passkeyNoticeLabel(view.prompt)}${view.count > 1 ? ` · ${view.count} requests` : ""} · F8 · ${countdown}` : ""
      const frame = JSON.stringify([{...view, remainingMs: undefined}, dimensions, theme.text, theme.primary, theme.backgroundPanel, theme.backgroundElement])
      if (lastFrame === frame) {
        // Clock ticks update text in place, preserving scroll and input focus.
        if (countdownText) countdownText.content = countdown
        if (noticeText) noticeText.content = notice
        box.requestRender()
        return
      }
      lastFrame = frame
      body = undefined
      countdownText = undefined
      noticeText = undefined
      for (const child of [...box.getChildren()]) {
        box.remove(String(child.id))
        child.destroyRecursively()
      }
      if (banner) {
        for (const child of [...banner.getChildren()]) {
          banner.remove(String(child.id))
          child.destroyRecursively()
        }
        banner.visible = view.count > 0
        banner.backgroundColor = theme.backgroundElement
        banner.paddingLeft = 1
        banner.paddingRight = 1
        if (view.count) { noticeText = text(notice, {bold: true}); banner.add(noticeText) }
        banner.onMouseUp = (event) => {
          event.stopPropagation()
          if (event.button === MouseButton.LEFT) actions.show()
        }
        banner.requestRender()
      }
      box.visible = view.count > 0 && (view.open || !banner)
      if (!view.count || !view.prompt) {
        box.requestRender()
        return
      }
      box.width = dimensions.width
      if (!view.open) {
        if (banner) { box.requestRender(); return }
        box.height = 2
        const indicator = new BoxRenderable(renderer, {
          position: "absolute", right: 0, top: 1, height: 1,
          backgroundColor: theme.backgroundElement, paddingLeft: 1, paddingRight: 1,
        })
        noticeText = text(notice, {accent: true})
        indicator.add(noticeText)
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
      const showDetails = Boolean(view.detailsOpen)
      const safe = (value: string) => value.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "").replace(/[\x00-\x08\x0b-\x1f\x7f-\x9f\u202a-\u202e\u2066-\u2069]/g, "")
      // The request scrolls when the terminal is too short for all of it.
      const fixedRows = 15 + (prompt.sudo_request ? 2 : 0) + (showDetails ? 8 : 0) + (view.error ? 2 : 0)
      const access = prompt.kind === "access_grant" || prompt.kind === "access_extension"
      const kernelAccess = access && prompt.session_id === "kernel-access"
      const showKind = !access || view.count > 1
      const wanted = (prompt.sudo_request ? 5 : 0) + (showKind ? 2 : 1) + rows(prompt.title || "Critical approval", width - 5) + rows(prompt.message, width - 5)
      body = new ScrollBoxRenderable(renderer, {
        height: Math.max(2, Math.min(wanted, dimensions.height - top - 1 - fixedRows)),
        flexShrink: 0, scrollY: true, scrollX: false,
      })
      if (showKind) body.add(text(`${prompt.kind === "critical_approval" ? "Critical approval" : prompt.kind === "sudo" ? "Sudo window" : "External agent access"}${view.count > 1 ? ` · ${view.index + 1} of ${view.count}` : ""}`, { muted: true }))
      body.add(text(prompt.title || "Critical approval", { accent: true, bold: true }))
      body.add(text(prompt.message))
      if (prompt.sudo_request) {
        const sudo = prompt.sudo_request
        body.add(text(`Agent: ${safe(sudo.agent_name)} · Session: ${safe(sudo.session_name)}`))
        body.add(text(`“${safe(sudo.task_summary)}”`))
      }
      panel.add(body)
      countdownText = text(countdown, {muted: true})
      section(countdownText)
      const session = prompt.session_alias ? `${prompt.session_alias} (${prompt.session_id})` : prompt.session_id
      if (prompt.sudo_request) {
        const sudo = prompt.sudo_request
        const toggle = button(showDetails ? "Hide details · Ctrl+D" : "Show details · Ctrl+D", true, () => {
          actions.toggleDetails?.()
        })
        section(toggle)
        if (showDetails) {
          const details = new ScrollBoxRenderable(renderer, {height: Math.max(2, Math.min(7, dimensions.height - top - 23)), flexShrink: 0, scrollY: true, scrollX: false})
          details.add(text(safe(`Requested task:\n${sudo.task}\nSession ID: ${prompt.session_id}\n${prompt.requester ? `PID: ${prompt.requester.pid}\nExecutable: ${prompt.requester.executable}\n` : ""}${sudo.grant_id ? `Grant ID: ${sudo.grant_id}\n` : ""}Terminal ID: ${sudo.terminal_id}`)))
          panel.add(details)
          body = details // PageUp/PageDown scroll the expanded details.
        }
      }
      if (!kernelAccess && !prompt.sudo_request) section(text(`${prompt.session_id === "kernel-access" && (prompt.kind === "access_grant" || prompt.kind === "access_extension")
        ? "Local kernel" : prompt.session_id ? `Session: ${session}` : "User domain"} · expires ${expiry(prompt.expires_at_ms)}`, { muted: true }))
      // Hidden input: only the length is ever rendered.
      const remember = text(prompt.kind === "critical_approval"
        ? `Remember for: ${view.passkey.rememberMinutes ? `${view.passkey.rememberMinutes} minutes` : "off"}`
        : prompt.kind === "sudo" ? `Window: ${hours(view.passkey.lifetime)} · maximum ${hours(prompt.max_lifetime_minutes)} · fresh passkey required`
        : `Access for: ${view.passkey.lifetime === UNTIL_AGENT_EXITS ? "until the agent exits" : `${view.passkey.lifetime} minutes`}`)
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
        : `Enter approves · Ctrl+R refuses${view.prompt?.kind === "critical_approval" ? " · Tab remember" : view.prompt?.kind === "sudo" ? " · Tab duration" : " · Tab lifetime"}${view.count > 1 ? " · ←/→ requests" : ""}`, { muted: true }))
      scrim.add(panel)
      box.add(scrim)
      box.requestRender()
    },
  }
}
