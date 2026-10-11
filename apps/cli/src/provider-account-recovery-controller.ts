import { BoxRenderable, TextRenderable } from "@opentui/core"
import type { CliStdinKeyEvent } from "./cli-stdin-key-controller.js"
import { theme } from "./theme.js"

export type ProviderAccountRecoveryTarget = {sessionId: string; agentId: string}

const quotaRefusal = /has exhausted its allowance|cannot start new work because its usage (?:allowance and credits are|is) exhausted/i

type RecoveryAction = { label: string; run(): void }

export function createProviderAccountRecoveryController(options: {
  renderer: ConstructorParameters<typeof BoxRenderable>[0]
  dimensions(): { width: number; height: number }
  actions(target?: ProviderAccountRecoveryTarget): RecoveryAction[]
}) {
  let box: BoxRenderable | undefined
  let message: string | null = null
  let target: ProviderAccountRecoveryTarget | undefined
  let customActions: RecoveryAction[] | null = null
  let customTitle: string | null = null
  const actions = () => customActions ?? options.actions(target)
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
    customActions = null
    customTitle = null
    // The same key also reaches the raw stdin listener during this event turn.
    closing = true
    setTimeout(() => { closing = false }, 0)
    clear()
    box?.requestRender()
  }
  const invoke = (index: number) => {
    const action = actions()[index]
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
    panel.add(new TextRenderable(options.renderer, { content: customTitle ?? (message.includes("sign-in not checked") ? "Check provider account sign-in" : quotaRefusal.test(message) ? "Choose provider account with quota" : "Provider account needs sign-in"), fg: theme.text }))
    panel.add(new TextRenderable(options.renderer, { content: message, fg: theme.text, wrapMode: "word" }))
    actions().forEach((action, index) => {
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
    showLaunchRefusal(messageText: string, retry: () => void) {
      message = messageText
      target = undefined
      customTitle = "Session launch refused"
      customActions = [{label: "Retry", run: retry}]
      selected = 0
      render()
    },
    showAccountChoices(messageText: string, choices: RecoveryAction[]) {
      if (!choices.length) return
      message = messageText
      target = undefined
      customTitle = "Choose provider account"
      customActions = choices
      selected = 0
      render()
    },
    show(messageText: string, rejectedTarget?: ProviderAccountRecoveryTarget) {
      if (!/selected \S+ account .*not authenticated|needs sign-in\.|sign-in not checked\./i.test(messageText) && !quotaRefusal.test(messageText)) return
      customActions = null
    customTitle = null
      target = rejectedTarget
      message = messageText
      selected = 0
      render()
    },
    handleKey(event: CliStdinKeyEvent): boolean {
      if (closing) return true
      if (!message) return closing
      if (event.eventType === "release") return true
      if (event.name === "escape") close()
      else if (event.name === "return" || event.name === "enter") invoke(selected)
      else if (event.name === "down" || event.name === "tab") {
        selected = (selected + 1) % actions().length; render()
      } else if (event.name === "up") {
        selected = (selected + actions().length - 1) % actions().length; render()
      }
      return true
    },
    close,
    render,
  }
}
