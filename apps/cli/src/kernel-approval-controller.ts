import type { RuntimeInteraction, RuntimeSession } from "./cli-types.js"
import type { InteractionPasskeyProof } from "./ipc-requests.js"

/** Optional remember window after a verified passkey: off, 5 or 15 minutes. */
export const PASSKEY_REMEMBER_MINUTES = [0, 5, 15] as const
const PASSKEY_MAX_LENGTH = 512

/** Fixed text for the kernel's passkey refusals; other errors stay generic. */
const PASSKEY_REFUSALS: Record<string, string> = {
  PASSKEY_REJECTED: "That passkey is not correct.",
  PASSKEY_RATE_LIMITED: "Too many wrong passkeys. Wait before trying again.",
  PASSKEY_UNAVAILABLE: "Critical approvals need the Chariox vault and its passphrase.",
  PASSKEY_REQUIRED: "Enter your Chariox passkey to approve.",
}

function passkeyRefusal(error: unknown): string | null {
  const text = error instanceof Error ? error.message : String(error)
  return Object.keys(PASSKEY_REFUSALS).find((code) => text.includes(code)) ?? null
}

export type KernelApprovalKey = {
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

export type KernelApprovalView = {
  open: boolean
  count: number
  index: number
  interaction: RuntimeInteraction | null
  selected: number | null
  pending: boolean
  connected: boolean
  error: string | null
  /** Hidden passkey entry for a critical approval: only its length is shown. */
  passkey: { length: number; rememberMinutes: number } | null
}

export function kernelApprovals(session: RuntimeSession): RuntimeInteraction[] {
  return (session.active_interactions ?? []).filter((item) =>
    typeof item.kernel_operation_id === "string" && item.kernel_operation_id.trim().length > 0
    && !Object.hasOwn(item, "agent_id") && item.kind === "permission"
    && item.custom_choice == null && item.default_on_timeout == null,
  ).sort((a, b) => a.requested_at_ms - b.requested_at_ms || a.id.localeCompare(b.id))
}

export function createKernelApprovalController(deps: {
  getSession(): RuntimeSession
  connected(): boolean
  onView(view: KernelApprovalView): void
  onOpen(): void
  onClose(): void
  scroll(direction: -1 | 1): void
  respond(sessionId: string, interactionId: string, choiceId: string, proof?: InteractionPasskeyProof): Promise<RuntimeSession>
  applySession(session: RuntimeSession): void
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
  // The passkey lives only here until it is sent, then it is dropped.
  let entry: { interactionId: string; choiceId: string; value: string; remember: number } | null = null
  // This terminal's own remember window; the kernel remains the authority.
  let rememberedUntil = 0
  const view = (): KernelApprovalView => {
    const items = kernelApprovals(deps.getSession())
    index = Math.min(index, Math.max(0, items.length - 1))
    const interaction = items[index] ?? null
    if (entry && entry.interactionId !== interaction?.id) entry = null
    return { open, count: items.length, index, interaction,
      selected, pending: pending !== null, connected: deps.connected(), error,
      passkey: entry ? { length: entry.value.length, rememberMinutes: entry.remember } : null }
  }
  const render = () => { if (!disposed) deps.onView(view()) }
  const close = () => {
    if (!open) return
    open = false
    selected = null
    entry = null
    deps.onClose()
    render()
  }
  const sync = () => {
    const session = deps.getSession()
    if (sessionId !== session.id) {
      sessionId = session.id
      epoch += 1
      pending = null
      index = 0
      error = null
      close()
    }
    const nextIdentity = JSON.stringify(view().interaction)
    if (identity !== nextIdentity) {
      identity = nextIdentity
      selected = null
      error = null
      entry = null
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
  const openEntry = (interactionId: string, choiceId: string) => {
    entry = { interactionId, choiceId, value: "", remember: 0 }
    render()
  }
  const send = async (interactionId: string, choiceId: string, proof?: InteractionPasskeyProof) => {
    const requestEpoch = epoch
    const requestSession = sessionId
    const token = {}
    pending = token
    error = null
    render()
    try {
      const response = await (proof
        ? deps.respond(requestSession, interactionId, choiceId, proof)
        : deps.respond(requestSession, interactionId, choiceId))
      if (proof?.rememberMinutes) rememberedUntil = Date.now() + proof.rememberMinutes * 60_000
      if (!disposed && requestEpoch === epoch && deps.getSession().id === requestSession) {
        if (response.id !== requestSession) throw new Error("interaction response session mismatch")
        deps.applySession(response)
      }
    } catch (failure) {
      if (!disposed && requestEpoch === epoch && deps.getSession().id === requestSession) {
        const refusal = passkeyRefusal(failure)
        error = refusal ? PASSKEY_REFUSALS[refusal]!
          : "The kernel did not confirm this choice. Check the pending approval before retrying."
        if (refusal && !proof) rememberedUntil = 0
        // A refused critical approval asks for the passkey (again).
        if (refusal && refusal !== "PASSKEY_UNAVAILABLE") openEntry(interactionId, choiceId)
      }
    } finally {
      if (pending === token) pending = null
      if (!disposed) sync()
    }
  }
  const choose = async (interactionId: string, choiceId: string) => {
    sync()
    const current = view()
    const choice = current.interaction?.choices.find((item) => item.id === choiceId)
    if (!current.open || !current.connected || pending || !current.interaction
      || current.interaction.id !== interactionId || !choice) return
    if (choice.requires_passkey && Date.now() >= rememberedUntil) {
      error = null
      openEntry(interactionId, choiceId)
      return
    }
    await send(interactionId, choiceId)
  }
  const submitPasskey = async () => {
    sync()
    const current = view()
    if (!entry || !entry.value || !current.connected || pending) return
    const { interactionId, choiceId, value, remember } = entry
    entry = null
    await send(interactionId, choiceId, { passkey: value, rememberMinutes: remember || null })
  }
  const cycleRemember = () => {
    if (!entry) return
    const options: readonly number[] = PASSKEY_REMEMBER_MINUTES
    entry.remember = options[(options.indexOf(entry.remember) + 1) % options.length]!
    render()
  }
  const appendPasskey = (text: string) => {
    if (!entry || /[\u0000-\u001f\u007f-\u009f]/.test(text)) return
    for (const character of text) {
      if (entry.value.length + character.length > PASSKEY_MAX_LENGTH) break
      entry.value += character
    }
    render()
  }
  const passkeyKey = (event: KernelApprovalKey) => {
    if (!entry) return
    if (event.name === "escape") { entry = null; error = null; render(); return }
    if (event.name === "return" || event.name === "enter") { void submitPasskey(); return }
    if (event.name === "tab") { cycleRemember(); return }
    if (event.name === "backspace") { entry.value = entry.value.slice(0, -1); render(); return }
    const typed = event.sequence && !/[\u0000-\u001f\u007f-\u009f]/.test(event.sequence)
      ? event.sequence
      : event.name === "space" ? " "
      : event.name.length === 1 ? (event.shift ? event.name.toUpperCase() : event.name) : ""
    if (typed) appendPasskey(typed)
  }
  return {
    sync, view, show, close, choose, submitPasskey, cycleRemember,
    isOpen: () => open,
    ownsInput: () => open || claimedInputTurn,
    dispose() { disposed = true; epoch += 1; close() },
    handlePaste(event: { text: string; defaultPrevented?: boolean; preventDefault(): void; stopPropagation(): void }): boolean {
      if (event.defaultPrevented) return true
      if (!open) return false
      event.preventDefault()
      event.stopPropagation()
      claimedInputTurn = true
      queueMicrotask(() => { claimedInputTurn = false })
      sync()
      if (!pending && entry) appendPasskey(event.text)
      return true
    },
    handleKey(event: KernelApprovalKey): boolean {
      if (event.defaultPrevented) return true
      if (!open && event.name !== "f8") return false
      if (!open && !view().count) return false
      event.preventDefault()
      event.stopPropagation()
      claimedInputTurn = true
      queueMicrotask(() => { claimedInputTurn = false })
      if (event.eventType === "release" || event.eventType === "repeat") return true
      if (event.name === "f8" || (event.name === "escape" && !entry)) {
        if (open) close(); else show()
        return true
      }
      if (event.ctrl || event.meta || event.alt) return true
      sync()
      const current = view()
      if (current.pending || !current.interaction) return true
      if (entry) {
        passkeyKey(event)
        return true
      }
      if (event.name === "left" || event.name === "right") {
        index = (index + (event.name === "left" ? -1 : 1) + current.count) % current.count
        sync()
      } else if (event.name === "up" || event.name === "down" || event.name === "tab") {
        const count = current.interaction.choices.length
        if (count) {
          const step = event.name === "up" || event.shift ? -1 : 1
          selected = selected === null ? (step === 1 ? 0 : count - 1)
            : (selected + step + count) % count
          render()
        }
      } else if (event.name === "pageup" || event.name === "pagedown") {
        deps.scroll(event.name === "pageup" ? -1 : 1)
      } else if ((event.name === "return" || event.name === "enter") && selected !== null) {
        const choice = current.interaction.choices[selected]
        if (choice) void choose(current.interaction.id, choice.id)
      }
      return true
    },
  }
}
