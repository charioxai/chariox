import type { PasskeyPrompt } from "@chariox/kernel-client/kernel-types"
import type { InteractionPasskeyProof } from "./ipc-requests.js"

/** Optional remember window after a verified passkey: off, 5 or 15 minutes. */
export const PASSKEY_REMEMBER_MINUTES = [0, 5, 15] as const
export const SUDO_WINDOW_MINUTES = [60, 120, 240, 480] as const
const PASSKEY_MAX_LENGTH = 512
const CONTROL_CHARACTERS = /[\u0000-\u001f\u007f-\u009f]/
const PASSKEY_PASTE_REFUSED = `Paste not added: a passkey is one line of at most ${PASSKEY_MAX_LENGTH} characters, without control characters.`
const PASSKEY_INPUT_REFUSED = "The passkey was cleared: the input held a terminal control sequence. Enter it again."
const UNCONFIRMED = "The kernel did not confirm this answer. Try again."

/** Fixed text for the kernel's passkey refusals; other errors stay generic. */
const PASSKEY_REFUSALS: Record<string, string> = {
  PASSKEY_ALREADY_ANSWERED: "This passkey request was already answered.",
  PASSKEY_REJECTED: "That passkey is not correct.",
  PASSKEY_RATE_LIMITED: "Too many wrong passkeys. Wait before trying again.",
  PASSKEY_UNAVAILABLE: "This approval needs the encrypted Chariox vault and its passphrase.",
  PASSKEY_REQUIRED: "Enter your Chariox passkey to approve.",
  PASSKEY_NOT_ACCEPTED: "Only a Chariox terminal can submit the passkey.",
}

function passkeyRefusal(error: unknown): string | null {
  const text = error instanceof Error ? error.message : String(error)
  return Object.keys(PASSKEY_REFUSALS).find((code) => text.includes(code)) ?? null
}

export type PasskeyPopupKey = {
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

export type PasskeyPopupPaste = {
  text: string
  /** The paste as the terminal sent it, before OpenTUI strips ANSI codes. */
  rawText?: string | null
  preventDefault(): void
  stopPropagation(): void
}

export type PasskeyPopupView = {
  /** The popup is shown; otherwise pending prompts only show an indicator. */
  open: boolean
  count: number
  index: number
  prompt: PasskeyPrompt | null
  /** Hidden entry: only the passkey's length ever leaves the controller. */
  passkey: { length: number; rememberMinutes: number; accessLifetimeMinutes?: number | null }
  pending: boolean
  connected: boolean
  error: string | null
}

const isText = (value: unknown): value is string => typeof value === "string" && value.trim().length > 0

/** The well-formed prompts of a `passkey_prompts_changed` event. */
export function passkeyPromptsFromEvent(prompts: unknown): PasskeyPrompt[] {
  if (!Array.isArray(prompts)) return []
  return prompts.filter((item): item is PasskeyPrompt => {
    const prompt = item as Partial<PasskeyPrompt> | null
    return !!prompt && ["critical_approval", "access_grant", "access_extension", "sudo"].includes(prompt.kind ?? "")
      && isText(prompt.session_id) && isText(prompt.interaction_id)
      && typeof prompt.title === "string" && typeof prompt.message === "string"
      && isText(prompt.approve_choice_id) && isText(prompt.refuse_choice_id)
      && prompt.approve_choice_id !== prompt.refuse_choice_id
      && typeof prompt.requested_at_ms === "number" && typeof prompt.expires_at_ms === "number"
  })
}

/** The text a key press types into the passkey, exactly as typed: its
 * `sequence` keeps case, shifted symbols and characters outside the BMP. A
 * kitty key without associated text falls back to its one-character name. */
function passkeyKeyText(event: PasskeyPopupKey): string {
  if (event.sequence && !CONTROL_CHARACTERS.test(event.sequence)) return event.sequence
  if (event.name === "space") return " "
  if (Array.from(event.name).length !== 1 || CONTROL_CHARACTERS.test(event.name)) return ""
  return event.shift ? event.name.toLocaleUpperCase() : event.name
}

const promptKey = (prompt: PasskeyPrompt) => `${prompt.session_id}\u001f${prompt.interaction_id}`

/** Protocol 403: the kernel's passkey prompts as one popup on this terminal,
 * attached to a session or not. The passkey is typed only here. The kernel
 * closes a prompt everywhere once any terminal answers it. */
export function createPasskeyPopupController(deps: {
  connected(): boolean
  onView(view: PasskeyPopupView): void
  onOpen(): void
  onClose(): void
  scroll(direction: -1 | 1): void
  respond(prompt: PasskeyPrompt, choiceId: string, proof?: InteractionPasskeyProof): Promise<unknown>
  notify(message: string): void
}) {
  let prompts: PasskeyPrompt[] = []
  let index = 0
  let hidden = true
  let open = false
  // The passkey lives only here until it is sent, then it is dropped.
  let value = ""
  let remember = 0
  let accessLifetime: number | null = null
  // This terminal's own remember window; the kernel remains the authority.
  let rememberedUntil = 0
  let pending: object | null = null
  let error: string | null = null
  let disposed = false
  let claimedInputTurn = false
  // Set for the rest of one terminal read after a refused control sequence.
  let refusingInput = false
  // Answered here; hidden until the kernel's set drops them too.
  const answered = new Set<string>()

  const current = () => prompts[index] ?? null
  const view = (): PasskeyPopupView => ({
    open, count: prompts.length, index, prompt: current(),
    passkey: { length: Array.from(value).length, rememberMinutes: remember, ...(current()?.kind !== "critical_approval" && current()?.lifetime_minutes ? { accessLifetimeMinutes: accessLifetime ?? current()?.lifetime_minutes ?? null } : {}) },
    pending: pending !== null, connected: deps.connected(), error,
  })
  const render = () => { if (!disposed) deps.onView(view()) }
  const clearEntry = () => { value = ""; remember = 0; accessLifetime = null; error = null }
  const syncOpen = () => {
    const next = prompts.length > 0 && !hidden && !disposed
    if (next === open) return
    open = next
    if (open) deps.onOpen()
    else { clearEntry(); deps.onClose() }
  }
  // The typed passkey belongs to the request it was typed for: any change of
  // the shown request drops it, even when another one takes the same place.
  const select = (next: number, shown: PasskeyPrompt | null = current()) => {
    index = next
    const now = current()
    if (!shown || !now || promptKey(shown) !== promptKey(now)) clearEntry()
  }
  // Keeps the shown request where it went, or shows the first one.
  const reselect = (shown: PasskeyPrompt | null) => {
    const at = shown ? prompts.findIndex((item) => promptKey(item) === promptKey(shown)) : -1
    // A different request needs deliberate focus, including when a remembered
    // approval closes and another pending request takes its place.
    if (at < 0) hidden = true
    select(at >= 0 ? at : 0, shown)
  }
  const settle = (prompt: PasskeyPrompt) => {
    const key = promptKey(prompt)
    answered.add(key)
    const shown = current()
    prompts = prompts.filter((item) => promptKey(item) !== key)
    reselect(shown)
    syncOpen()
  }
  const send = async (prompt: PasskeyPrompt, choiceId: string, proof?: InteractionPasskeyProof) => {
    const token = {}
    pending = token
    error = null
    render()
    try {
      await (proof ? deps.respond(prompt, choiceId, proof) : deps.respond(prompt, choiceId))
      if (proof?.rememberMinutes) rememberedUntil = Date.now() + proof.rememberMinutes * 60_000
      settle(prompt)
    } catch (failure) {
      const refusal = passkeyRefusal(failure)
      if (refusal === "PASSKEY_ALREADY_ANSWERED") {
        settle(prompt)
        deps.notify(PASSKEY_REFUSALS[refusal]!)
      } else {
        if (refusal === "PASSKEY_REQUIRED") rememberedUntil = 0
        if (current() && promptKey(current()!) === promptKey(prompt)) {
          error = refusal ? PASSKEY_REFUSALS[refusal]! : UNCONFIRMED
        }
      }
    } finally {
      if (pending === token) pending = null
      render()
    }
  }
  const approve = async () => {
    const prompt = current()
    if (!open || !prompt || pending || !deps.connected()) return
    const typed = value
    if (!typed && (prompt.kind !== "critical_approval" || Date.now() >= rememberedUntil)) {
      error = PASSKEY_REFUSALS.PASSKEY_REQUIRED!
      render()
      return
    }
    const minutes = prompt.kind === "critical_approval" ? remember : 0
    value = ""
    await send(prompt, prompt.approve_choice_id,
      typed ? { passkey: typed, rememberMinutes: minutes || null, ...(["access_grant", "access_extension", "sudo"].includes(prompt.kind) ? { accessLifetimeMinutes: accessLifetime ?? prompt.lifetime_minutes } : {}) } : undefined)
  }
  const refuse = async () => {
    const prompt = current()
    if (!open || !prompt || pending || !deps.connected()) return
    value = ""
    await send(prompt, prompt.refuse_choice_id)
  }
  const cycleRemember = () => {
    if (!open) return
    const prompt = current()
    if (prompt && prompt.kind !== "critical_approval") {
      const max = prompt.max_lifetime_minutes
      if (!max || !prompt.lifetime_minutes) return
      // Sudo windows: one hour by default, 2, 4 or 8 hours per fresh passkey.
      const offered = prompt.kind === "sudo" ? SUDO_WINDOW_MINUTES : [prompt.lifetime_minutes, 5, 15, 30, 60, 120, max]
      const options = [...new Set(offered)].filter(m => m <= max).sort((a,b) => a-b)
      accessLifetime = options[(options.indexOf(accessLifetime ?? prompt.lifetime_minutes) + 1) % options.length]!
      render()
      return
    }
    const options: readonly number[] = PASSKEY_REMEMBER_MINUTES
    remember = options[(options.indexOf(remember) + 1) % options.length]!
    render()
  }
  const hide = () => {
    hidden = true
    syncOpen()
    render()
  }
  return {
    view, approve, refuse, cycleRemember, hide,
    isOpen: () => open,
    ownsInput: () => open || claimedInputTurn,
    /** The kernel's current set, from `passkey_prompts_changed`. A prompt
     * that arrives shows an indicator; F8 or a click explicitly opens it. */
    apply(next: PasskeyPrompt[]) {
      const shown = current()
      const listed = new Set(next.map(promptKey))
      for (const key of answered) if (!listed.has(key)) answered.delete(key)
      prompts = next.filter((prompt) => !answered.has(promptKey(prompt)))
      reselect(shown)
      syncOpen()
      render()
    },
    /** Shows the popup on this prompt; false when this terminal has none. */
    show(sessionId?: string, interactionId?: string): boolean {
      const at = sessionId === undefined ? index
        : prompts.findIndex((prompt) => prompt.session_id === sessionId && prompt.interaction_id === interactionId)
      if (at < 0 || !prompts.length) return false
      select(at)
      hidden = false
      syncOpen()
      render()
      return true
    },
    handlePaste(event: PasskeyPopupPaste): boolean {
      if (!open) return false
      event.preventDefault()
      event.stopPropagation()
      if (pending) return true
      // Added exactly as sent, without a trailing line break, or refused
      // whole: OpenTUI strips ANSI codes from `text`, so the raw paste decides.
      const text = (event.rawText ?? event.text).replace(/(?:\r\n|\r|\n)+$/, "")
      if (CONTROL_CHARACTERS.test(text) || Array.from(value + text).length > PASSKEY_MAX_LENGTH) {
        error = PASSKEY_PASTE_REFUSED
      } else {
        value += text
        error = null
      }
      render()
      return true
    },
    handleKey(event: PasskeyPopupKey): boolean {
      if (event.defaultPrevented) return true
      if (!open) {
        // F8 brings a hidden popup back before the approval panel.
        if (event.name !== "f8" || !prompts.length) return false
        event.preventDefault()
        event.stopPropagation()
        if (event.eventType !== "release" && event.eventType !== "repeat") {
          hidden = false
          syncOpen()
          render()
        }
        return true
      }
      event.preventDefault()
      event.stopPropagation()
      claimedInputTurn = true
      queueMicrotask(() => { claimedInputTurn = false })
      if (event.eventType === "release" || event.eventType === "repeat") return true
      // An escape sequence that is no key is styled text pasted without
      // bracketed paste: refuse that input rather than drop its codes.
      if (!event.name && event.sequence && CONTROL_CHARACTERS.test(event.sequence)) {
        value = ""
        error = PASSKEY_INPUT_REFUSED
        refusingInput = true
        queueMicrotask(() => { refusingInput = false })
        render()
        return true
      }
      if (refusingInput) return true
      if (event.name === "escape" || event.name === "f8") { hide(); return true }
      if (event.name === "pageup" || event.name === "pagedown") {
        deps.scroll(event.name === "pageup" ? -1 : 1)
        return true
      }
      if (pending) return true
      if (event.ctrl && event.name === "r") { void refuse(); return true }
      if (event.ctrl || event.meta || event.alt) return true
      if (event.name === "return" || event.name === "enter") { void approve(); return true }
      if (event.name === "tab") { cycleRemember(); return true }
      if (event.name === "left" || event.name === "right") {
        if (prompts.length > 1) {
          select((index + (event.name === "left" ? -1 : 1) + prompts.length) % prompts.length)
          render()
        }
        return true
      }
      if (event.name === "backspace") {
        value = Array.from(value).slice(0, -1).join("")
        render()
        return true
      }
      const typed = passkeyKeyText(event)
      if (typed && Array.from(value + typed).length <= PASSKEY_MAX_LENGTH) {
        value += typed
        render()
      }
      return true
    },
    dispose() {
      disposed = true
      clearEntry()
      if (open) { open = false; deps.onClose() }
    },
  }
}
