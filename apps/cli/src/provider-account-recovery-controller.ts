import { BoxRenderable, TextRenderable } from "@opentui/core"
import type { CliStdinKeyEvent } from "./cli-stdin-key-controller.js"
import { theme } from "./theme.js"

type RecoveryAction = { label: string; run(): void }

export function createProviderAccountRecoveryController(options: {
  renderer: ConstructorParameters<typeof BoxRenderable>[0]
  dimensions(): { width: number; height: number }
  actions(): RecoveryAction[]
}) {
  let box: BoxRenderable | undefined
  let message: string | null = null
  let selected = 0
  let closing = false
  const clear = () => {
    if (!box) return
    for (const child of [...box.getChildren()]) {
      box.remove(child.id)
      child.destroyRecursively()
    }
  }
  const close = () => {
    message = null
    // The same key also reaches the raw stdin listener during this event turn.
    closing = true
    setTimeout(() => { closing = false }, 0)
    clear()
    box?.requestRender()
  }
  const invoke = (index: number) => {
    const action = options.actions()[index]
    close()
    action?.run()
  }
  const render = () => {
    clear()
    if (!box || !message) return
    const size = options.dimensions()
    const width = Math.max(20, Math.min(92, size.width - 4))
    const panel = new BoxRenderable(options.renderer, {
      position: "absolute", left: Math.max(0, Math.floor((size.width - width) / 2)),
      top: Math.max(0, Math.floor(size.height / 5)), width,
      border: true, padding: 1, flexDirection: "column", backgroundColor: theme.background,
    })
    panel.add(new TextRenderable(options.renderer, { content: message.includes("sign-in not checked") ? "Check provider account sign-in" : message.includes("has exhausted") ? "Choose provider account with quota" : "Provider account needs sign-in", fg: theme.text }))
    panel.add(new TextRenderable(options.renderer, { content: message, fg: theme.text, wrapMode: "word" }))
    options.actions().forEach((action, index) => {
      const button = new TextRenderable(options.renderer, {
        content: `${selected === index ? ">" : " "} [${action.label}]`, fg: selected === index ? theme.primary : theme.text,
      })
      button.onMouseUp = event => { event.stopPropagation(); invoke(index) }
      panel.add(button)
    })
    panel.add(new TextRenderable(options.renderer, { content: "↑/↓ choose · Enter select · Esc close", fg: theme.textMuted }))
    box.add(panel)
    box.requestRender()
  }
  return {
    assignBox(value: BoxRenderable | undefined) { box = value; render() },
    ownsInput: () => message !== null || closing,
    show(messageText: string) {
      if (!/selected \S+ account .*not authenticated|needs sign-in\.|sign-in not checked\.|has exhausted its allowance/i.test(messageText)) return
      message = messageText
      selected = 0
      render()
    },
    handleKey(event: CliStdinKeyEvent): boolean {
      if (!message) return closing
      if (event.eventType === "release") return true
      if (event.name === "escape") close()
      else if (event.name === "return" || event.name === "enter") invoke(selected)
      else if (event.name === "down" || event.name === "tab") {
        selected = (selected + 1) % options.actions().length; render()
      } else if (event.name === "up") {
        selected = (selected + options.actions().length - 1) % options.actions().length; render()
      }
      return true
    },
    close,
    render,
  }
}
