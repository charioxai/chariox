import { isApprovalShortcut } from "./approval-shortcuts.js"
import type { HandoffOutcome, HandoffResponseAction } from "@chariox/kernel-client/owner-handoff"
import type { RuntimeInteraction, RuntimeInteractionChoice, RuntimeSession } from "./cli-types.js"
import {
  createHandoffEntry, handoffChoices, handoffKeyText, handoffResultText, interactionHandoff,
  HANDOFF_CLICK_CHOICE, HANDOFF_ENTER_CHOICE, type HandoffEntryView,
} from "./kernel-handoff-entry.js"

/** Protocol 403: the passkey is typed only into the passkey popup. */
const PASSKEY_IN_POPUP = "Approve this in the Chariox passkey popup. Only the decision's owner gets it."

export type KernelApprovalKey = {
  super?: boolean
  name: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  shift?: boolean
  sequence?: string
  eventType?: string
  defaultPrevented?: boolean
  preventDefault(): void
  stopPropagation(): void
}

export type KernelApprovalPaste = {
  text?: string
  rawText?: string | null
  preventDefault(): void
  stopPropagation(): void
}

export type KernelApprovalView = {
  open: boolean
  count: number
  criticalCount: number
  index: number
  interaction: RuntimeInteraction | null
  /** The interaction's choices; a hand-off adds its protected owner action. */
  choices: RuntimeInteractionChoice[]
  /** MP-11 A07: protected value entry; only the length leaves the controller. */
  handoffEntry: HandoffEntryView | null
  selected: number | null
  pending: boolean
  connected: boolean
  error: string | null
}

export function kernelApprovals(session: RuntimeSession): RuntimeInteraction[] {
  return (session.active_interactions ?? []).filter((item) =>
    typeof item.kernel_operation_id === "string" && item.kernel_operation_id.trim().length > 0
    && !Object.hasOwn(item, "agent_id") && item.kind === "permission"
    && item.custom_choice == null && item.default_on_timeout == null,
  ).sort((a, b) => a.requested_at_ms - b.requested_at_ms || a.id.localeCompare(b.id))
}

export function appHostOperationId(interaction: RuntimeInteraction): string | undefined {
  const operation = interaction.kernel_operation_id?.match(/^host_action:(.+)$/)?.[1]
  return operation && interaction.id === `app_host_${operation}` ? operation : undefined
}

export function createKernelApprovalController(deps: {
  getSession(): RuntimeSession
  connected(): boolean
  onView(view: KernelApprovalView): void
  onOpen(): void
  onClose(): void
  scroll(direction: -1 | 1): void
  respond(sessionId: string, interactionId: string, choiceId: string): Promise<RuntimeSession>
  /** MP-11 A07: the owner's protected hand-off answer. */
  respondHandoff(sessionId: string, interactionId: string, action: HandoffResponseAction): Promise<HandoffOutcome>
  notify(message: string): void
  applySession(session: RuntimeSession): void
  /** Shows this terminal's passkey popup for a critical approval; false when
   * it has none (another user's decision). */
  showPasskeyPrompt(sessionId: string, interactionId: string): boolean
}) {
  let sessionId = ""
  let epoch = 0
  let open = false
  let index = 0
  let selected: number | null = null
  let identity = ""
  let pending: object | null = null
  let error: string | null = null
  let disposed = false
  let claimedInputTurn = false
  let lastDisplayedHost: { sessionId: string; operation: string; identity: string } | null = null
  let entry: { interactionId: string; value: ReturnType<typeof createHandoffEntry> } | null = null
  const view = (): KernelApprovalView => {
    const items = kernelApprovals(deps.getSession())
    index = Math.min(index, Math.max(0, items.length - 1))
    const interaction = items[index] ?? null
    if (entry && entry.interactionId !== interaction?.id) { entry.value.clear(); entry = null }
    return { open, count: items.length, criticalCount: items.filter(item => item.choices.some(choice => choice.requires_passkey)).length, index, interaction,
      choices: interaction ? handoffChoices(interaction) : [], handoffEntry: entry?.value.view() ?? null,
      selected, pending: pending !== null, connected: deps.connected(), error }
  }
  const render = () => {
    if (disposed) return
    const current = view()
    deps.onView(current)
    if (current.open) {
      const operation = current.interaction && appHostOperationId(current.interaction)
      lastDisplayedHost = operation ? { sessionId, operation, identity: JSON.stringify(current.interaction) } : null
    }
  }
  const close = () => {
    if (!open) return
    open = false
    selected = null
    entry?.value.clear()
    entry = null
    deps.onClose()
    render()
  }
  const sync = () => {
    const session = deps.getSession()
    if (sessionId !== session.id) {
      lastDisplayedHost = null
      sessionId = session.id
      epoch += 1
      pending = null
      index = 0
      error = null
      close()
    }
    if (!deps.connected()) { entry?.value.clear(); entry = null }
    const nextIdentity = JSON.stringify(view().interaction)
    if (identity !== nextIdentity) {
      identity = nextIdentity
      selected = null
      error = null
    }
    if (!view().count) close()
    render()
  }
  const show = () => {
    sync()
    if (open || !view().count) return
    open = true
    selected = null // Opening never selects a default approval.
    deps.onOpen()
    render()
  }
  const send = async (interactionId: string, choiceId: string) => {
    const requestEpoch = epoch
    const requestSession = sessionId
    const token = {}
    pending = token
    error = null
    render()
    try {
      const response = await deps.respond(requestSession, interactionId, choiceId)
      if (!disposed && requestEpoch === epoch && deps.getSession().id === requestSession) {
        if (response.id !== requestSession) throw new Error("interaction response session mismatch")
        deps.applySession(response)
      }
    } catch {
      if (!disposed && requestEpoch === epoch && deps.getSession().id === requestSession) {
        error = "The kernel did not confirm this choice. Check the pending approval before retrying."
      }
    } finally {
      if (pending === token) pending = null
      if (!disposed) sync()
    }
  }
  const sendHandoff = async (interactionId: string, action: HandoffResponseAction) => {
    const requestEpoch = epoch
    const requestSession = sessionId
    const token = {}
    pending = token
    error = null
    render()
    try {
      const outcome = await deps.respondHandoff(requestSession, interactionId, action)
      deps.notify(handoffResultText(outcome, null))
    } catch (cause) {
      if (!disposed && requestEpoch === epoch && deps.getSession().id === requestSession) error = handoffResultText(null, cause)
    } finally {
      if (pending === token) pending = null
      if (!disposed) sync()
    }
  }
  const submitEntry = () => {
    const action = entry?.value.take()
    const interactionId = entry?.interactionId
    entry = null
    selected = null
    if (action && interactionId) void sendHandoff(interactionId, action)
    else render()
  }
  const choose = async (interactionId: string, choiceId: string) => {
    sync()
    const current = view()
    const choice = current.choices.find((item) => item.id === choiceId)
    if (!current.open || !current.connected || pending || !current.interaction
      || current.interaction.id !== interactionId || !choice) return
    const handoff = interactionHandoff(current.interaction)
    if (handoff && choiceId === HANDOFF_CLICK_CHOICE) return sendHandoff(interactionId, { kind: "click" })
    if (handoff && choiceId === HANDOFF_ENTER_CHOICE) {
      entry = { interactionId, value: createHandoffEntry(handoff) }
      error = null
      render()
      return
    }
    // A critical approval is approved only in the passkey popup.
    if (choice.requires_passkey) {
      error = deps.showPasskeyPrompt(sessionId, interactionId) ? null : PASSKEY_IN_POPUP
      render()
      return
    }
    await send(interactionId, choiceId)
  }
  // A paste never reaches the prompt while the panel is open.
  const handlePaste = (event: KernelApprovalPaste): boolean => {
    if (!open) return false
    event.preventDefault()
    event.stopPropagation()
    // In protected hand-off entry the paste is the value, refused whole when it
    // holds control characters; it never reaches the prompt.
    if (entry && !pending) {
      const text = (event.rawText ?? event.text ?? "").replace(/(?:\r\n|\r|\n)+$/, "")
      error = entry.value.add(text) ? null : "Paste not added: one line without control characters."
      render()
    }
    return true
  }
  return {
    sync, view, show, close, choose, handlePaste,
    lastViewedAppHostOperationId: (): string | undefined => {
      const session = deps.getSession()
      const displayed = lastDisplayedHost
      if (disposed || !displayed || session.id !== displayed.sessionId) return undefined
      const current = kernelApprovals(session).find(item => appHostOperationId(item) === displayed.operation)
      return current && JSON.stringify(current) === displayed.identity ? displayed.operation : undefined
    },
    isOpen: () => open,
    ownsInput: () => open || claimedInputTurn,
    dispose() { disposed = true; epoch += 1; entry?.value.clear(); entry = null; close() },
    handleKey(event: KernelApprovalKey): boolean {
      if (event.defaultPrevented) return true
      const shortcut = isApprovalShortcut(event)
      if (!open && !shortcut) return false
      if (!open && !view().count) return false
      event.preventDefault()
      event.stopPropagation()
      claimedInputTurn = true
      queueMicrotask(() => { claimedInputTurn = false })
      if (event.eventType === "release" || event.eventType === "repeat") return true
      if (shortcut || (event.name === "escape" && !entry)) {
        if (open) close(); else show()
        return true
      }
      if (entry && event.name === "escape") { entry.value.clear(); entry = null; render(); return true }
      if (event.ctrl || event.meta || event.alt) return true
      sync()
      const current = view()
      if (current.pending || !current.interaction) return true
      if (entry) {
        if (event.name === "return" || event.name === "enter") submitEntry()
        else if (event.name === "backspace") { entry.value.backspace(); render() }
        else if (event.name === "tab") { entry.value.toggleSave(); render() }
        else if (!entry.value.add(handoffKeyText(event))) render()
        else { error = null; render() }
        return true
      }
      if (event.name === "left" || event.name === "right") {
        index = (index + (event.name === "left" ? -1 : 1) + current.count) % current.count
        sync()
      } else if (event.name === "up" || event.name === "down" || event.name === "tab") {
        const count = current.choices.length
        if (count) {
          const step = event.name === "up" || event.shift ? -1 : 1
          selected = selected === null ? (step === 1 ? 0 : count - 1)
            : (selected + step + count) % count
          render()
        }
      } else if (event.name === "pageup" || event.name === "pagedown") {
        deps.scroll(event.name === "pageup" ? -1 : 1)
      } else if ((event.name === "return" || event.name === "enter") && selected !== null) {
        const choice = current.choices[selected]
        if (choice) void choose(current.interaction.id, choice.id)
      }
      return true
    },
  }
}
