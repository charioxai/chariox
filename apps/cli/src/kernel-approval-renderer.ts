import { BoxRenderable, ScrollBoxRenderable, TextRenderable, MouseButton, TextAttributes, type CliRenderer } from "@opentui/core"
import type { KernelApprovalView } from "./kernel-approval-controller.js"
import { approvalShortcutLabel } from "./approval-shortcuts.js"
import { theme } from "./theme.js"
import { handoffChangeLines, handoffReasonLabel } from "@chariox/kernel-client/owner-handoff"
import { interactionHandoff } from "./kernel-handoff-entry.js"
import { requesterLabels } from "./kernel-access-requester-label.js"

export function createKernelApprovalRenderer(renderer: CliRenderer, actions: {
  show(): void
  choose(interactionId: string, choiceId: string): void
}, platform = process.platform) {
  const shortcutLabel = approvalShortcutLabel(platform)
  let banner: BoxRenderable | undefined
  let box: BoxRenderable | undefined
  let body: ScrollBoxRenderable | undefined
  let lastFrame = ""
  return {
    assignBanner(value: BoxRenderable) { banner = value; lastFrame = "" },
    assign(value: BoxRenderable) { box = value; lastFrame = "" },
    scroll(direction: -1 | 1) { body?.scrollBy(direction * 4) },
    render(view: KernelApprovalView, dimensions: { width: number; height: number }) {
      if (!box && !banner) return
      const frame = JSON.stringify([view, dimensions, theme.text, theme.primary, theme.backgroundPanel, theme.backgroundElement])
      if (lastFrame === frame) return
      lastFrame = frame
      if (banner) {
        for (const child of [...banner.getChildren()]) {
          banner.remove(String(child.id))
          child.destroyRecursively()
        }
        banner.visible = view.count > 0
        banner.backgroundColor = theme.backgroundElement
        banner.paddingLeft = 1
        banner.paddingRight = 1
        banner.onMouseUp = (event) => {
          event.stopPropagation()
          if (event.button === MouseButton.LEFT) actions.show()
        }
        if (view.count) {
          const fullTitle = (view.interaction?.title || "Kernel approval").replace(/[\x00-\x1f\x7f]/g, " ")
          const title = Array.from(fullTitle).length > 120 ? `${Array.from(fullTitle).slice(0, 120).join("")}…` : fullTitle
          banner.add(new TextRenderable(renderer, {
            content: `Action needed · ${view.count} approval${view.count === 1 ? "" : "s"} waiting: ${title}`,
            wrapMode: "word", flexShrink: 0, fg: theme.warning, attributes: TextAttributes.BOLD,
          }))
          if (view.criticalCount) banner.add(new TextRenderable(renderer, {
            content: `Critical — passkey needed${view.criticalCount > 1 ? ` (${view.criticalCount} approvals)` : ""}. Open to review and approve with your passkey.`,
            wrapMode: "word", fg: theme.error, attributes: TextAttributes.BOLD, flexShrink: 0,
          }))
          banner.add(new TextRenderable(renderer, {
            content: `Open: ${shortcutLabel} or /approvals`,
            wrapMode: "word", fg: theme.primary, flexShrink: 0,
          }))
        }
        banner.requestRender()
      }
      if (!box) return
      body = undefined
      for (const child of [...box.getChildren()]) {
        box.remove(String(child.id))
        child.destroyRecursively()
      }
      box.visible = view.count > 0
      if (!view.count) return
      box.width = dimensions.width
      box.height = view.open ? dimensions.height : 1
      const text = (parent: BoxRenderable, content: string, accent = false) => {
        const node = new TextRenderable(renderer, {
          content, wrapMode: "word", fg: accent ? theme.primary : theme.text,
          flexShrink: 0,
        })
        parent.add(node)
        return node
      }
      const indicator = new BoxRenderable(renderer, {
        position: "absolute", right: 0, top: 0, height: 1,
        backgroundColor: theme.backgroundElement, paddingLeft: 1, paddingRight: 1,
      })
      text(indicator, `Chariox · ${view.count} approval${view.count === 1 ? "" : "s"} · ${shortcutLabel}`, true)
      indicator.onMouseUp = (event) => {
        event.stopPropagation()
        if (event.button === MouseButton.LEFT) actions.show()
      }
      box.add(indicator)
      if (!view.open || !view.interaction) return
      const panel = new BoxRenderable(renderer, {
        position: "absolute", top: Math.min(2, Math.max(0, dimensions.height - 5)),
        left: Math.max(0, Math.floor((dimensions.width - 76) / 2)),
        width: Math.min(76, dimensions.width), height: Math.max(4, dimensions.height - 4),
        border: true, borderColor: theme.primary, backgroundColor: theme.backgroundPanel,
        paddingLeft: 1, paddingRight: 1, flexDirection: "column",
      })
      panel.onMouseUp = (event) => event.stopPropagation()
      text(panel, `Chariox approval ${view.index + 1} of ${view.count} · Esc to dismiss`, true)
      body = new ScrollBoxRenderable(renderer, {
        flexGrow: 1, flexShrink: 1, scrollY: true, scrollX: false,
      })
      panel.add(body)
      text(body, view.interaction.title || "Kernel approval")
      if (view.interaction.requester) {
        for (const label of requesterLabels(view.interaction.requester)) text(body, label)
      }
      const handoff = interactionHandoff(view.interaction)
      if (handoff) {
        // MP-11 A07: safe target metadata and the intended change only.
        text(body, `${handoffReasonLabel(handoff.reason)} · ${handoff.target.origin}${handoff.target.path}`, true)
        text(body, handoff.explanation)
        text(body, "To review the live page, use /cloud open on this kernel and open this hand-off.", true)
        const lines = handoffChangeLines(handoff)
        if (lines.length) {
          text(body, "Intended change (verify before acting):", true)
          for (const line of lines) text(body, line)
        }
        text(body, `Expires ${new Date(handoff.expires_at_ms).toLocaleTimeString()}. Only you can act; the agent sees only the outcome.`)
      } else {
        text(body, view.interaction.message)
      }
      if (view.interaction.choices.some(choice => choice.requires_passkey)) {
        text(body, "Critical — passkey needed. Select an approval choice to enter your passkey.", true)
      }
      view.choices.forEach((choice, index) => {
        const row = new BoxRenderable(renderer, {
          flexShrink: 0, backgroundColor: view.selected === index ? theme.backgroundElement : theme.backgroundPanel,
        })
        text(row, `${view.selected === index ? "›" : " "} ${choice.label}`, view.selected === index)
        row.onMouseUp = (event) => {
          event.stopPropagation()
          if (event.button === MouseButton.LEFT && view.connected && !view.pending) actions.choose(view.interaction!.id, choice.id)
        }
        body!.add(row)
      })
      if (view.handoffEntry) {
        const entry = view.handoffEntry
        text(body, `${entry.kind === "code" ? "Code" : "Secret"}: ${"•".repeat(Math.min(entry.length, 64))}${entry.length ? "" : " (type or paste)"}`, true)
        if (entry.saveOffered) text(body, `Save to Vault: ${entry.saveToVault ? "yes" : "no"} (Tab to change)`)
        text(body, "Enter sends it straight to the page field · Esc clears")
      }
      if (view.error) text(body, view.error)
      if (view.selected !== null && !view.pending) {
        const choice = view.choices[view.selected]
        if (choice) text(panel, `Selected: ${choice.label}`, true)
      }
      text(panel, view.pending ? "Waiting for kernel confirmation…"
        : !view.connected ? "Disconnected · reconnect to respond"
        : "↑/↓ select · Enter confirm · ←/→ approvals · PgUp/PgDn scroll")
      box.add(panel)
      box.requestRender()
    },
  }
}
