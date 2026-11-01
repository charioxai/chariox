import type { PasskeyPrompt, PasskeyPromptClosure } from "@chariox/kernel-client/kernel-types"
import type { InteractionPasskeyProof } from "./ipc-requests.js"
import { approvalExpiredMessage, type KernelClock } from "./kernel-clock.js"

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

/** What an access or sudo popup answer asks for: minutes, or (access grants only) until the agent exits. */
export type AccessLifetime = number | typeof UNTIL_AGENT_EXITS

export const UNTIL_AGENT_EXITS = "until_exit"

/** The lifetime a popup starts on: the kernel's sudo window, or the term of an access request. */
function startingLifetime(prompt: PasskeyPrompt): AccessLifetime | null {
  if (prompt.kind === "sudo") return prompt.lifetime_minutes ?? null
  if (prompt.term === "until_holder_exits") return UNTIL_AGENT_EXITS
  return prompt.term?.minutes ?? null
}

export type PasskeyPopupView = {
  /** The popup is shown; otherwise pending prompts only show an indicator. */
  open: boolean
  count: number
  index: number
  prompt: PasskeyPrompt | null
  /** Hidden entry: only the passkey's length ever leaves the controller. */
  passkey: { length: number; rememberMinutes: number; lifetime?: AccessLifetime | null }
  pending: boolean
  connected: boolean
  error: string | null
  detailsOpen?: boolean
  remainingMs?: number | null
}

const isText = (value: unknown): value is string => typeof value === "string" && value.trim().length > 0

/** The well-formed prompts of a `passkey_prompts_changed` event. */
export function passkeyPromptsFromEvent(prompts: unknown, userAppViews = false): PasskeyPrompt[] {
  if (!Array.isArray(prompts)) return []
  return prompts.filter((item): item is PasskeyPrompt => {
    const prompt = item as Partial<PasskeyPrompt> | null
    return !!prompt && ["critical_approval", "access_grant", "access_extension", "sudo"].includes(prompt.kind ?? "")
      && (isText(prompt.session_id) || (userAppViews && prompt.session_id === "" && prompt.kind === "critical_approval")) && isText(prompt.interaction_id)
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

const promptKey = (prompt: Pick<PasskeyPrompt, "session_id" | "interaction_id">) => `${prompt.session_id}\u001f${prompt.interaction_id}`

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
  serverClock?: KernelClock
}) {
  let prompts: PasskeyPrompt[] = []
  let index = 0
  let detailsOpen = false
  let hidden = true
  let open = false
  // The passkey lives only here until it is sent, then it is dropped.
  let value = ""
  let remember = 0
  let accessLifetime: AccessLifetime | null = null
  // This terminal's own remember window; the kernel remains the authority.
  let rememberedUntil = 0
  let connectionEpoch = 0
  let pending: object | null = null
  let pendingKey: string | null = null
  let expiryTimer: ReturnType<typeof setTimeout> | null = null
  let clockRead = false
  let authoritativeFeed = false
  const awaitingClosure = new Map<string, PasskeyPrompt["kind"]>()
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
    passkey: { length: Array.from(value).length, rememberMinutes: remember, ...(current() && current()!.kind !== "critical_approval" && startingLifetime(current()!) !== null ? { lifetime: accessLifetime ?? startingLifetime(current()!) } : {}) },
    detailsOpen,
    pending: pending !== null, connected: deps.connected(), error,
    remainingMs: current() && deps.serverClock?.now() != null ? Math.max(0, current()!.expires_at_ms - deps.serverClock.now()!) : null,
  })
  const render = () => { if (!disposed) deps.onView(view()) }
  const clearEntry = () => { detailsOpen = false; value = ""; remember = 0; accessLifetime = null; error = null }
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
  const refreshExpiry = () => {
    if (expiryTimer) clearTimeout(expiryTimer)
    expiryTimer = null
    if (disposed || !prompts.length || !deps.serverClock) return
    const now = deps.serverClock.now()
    if (now === null && !clockRead) {
      clockRead = true
      void deps.serverClock.synchronize().catch(() => {}).finally(() => {
        clockRead = false
        if (!disposed && deps.serverClock?.now() != null) { refreshExpiry(); render() }
      })
    }
    const shown = current()
    const expired = prompts.filter(prompt => now !== null && prompt.expires_at_ms <= now && promptKey(prompt) !== pendingKey)
    for (const prompt of expired) {
      answered.add(promptKey(prompt))
      if (!authoritativeFeed) deps.notify(approvalExpiredMessage(prompt.kind))
    }
    if (expired.length) {
      const keys = new Set(expired.map(promptKey))
      prompts = prompts.filter(prompt => !keys.has(promptKey(prompt)))
      reselect(shown)
      syncOpen()
    }
    if (prompts.length) {
      const delay = Math.min(1_000, ...prompts.map(prompt => now !== null && prompt.expires_at_ms > now ? prompt.expires_at_ms - now + 1 : 1_000))
      expiryTimer = setTimeout(() => { refreshExpiry(); render() }, delay)
    }
  }
  const send = async (prompt: PasskeyPrompt, choiceId: string, proof?: InteractionPasskeyProof) => {
    const epoch = connectionEpoch
    const token = {}
    pending = token
    pendingKey = promptKey(prompt)
    error = null
    render()
    try {
      await (proof ? deps.respond(prompt, choiceId, proof) : deps.respond(prompt, choiceId))
      if (epoch !== connectionEpoch) return
      settle(prompt)
    } catch (failure) {
      if (epoch !== connectionEpoch) return
      const refusal = passkeyRefusal(failure)
      if (refusal === "PASSKEY_ALREADY_ANSWERED") {
        settle(prompt)
        deps.notify(PASSKEY_REFUSALS[refusal]!)
      } else {
        if (current() && promptKey(current()!) === promptKey(prompt)) {
          error = refusal ? PASSKEY_REFUSALS[refusal]! : UNCONFIRMED
        }
      }
    } finally {
      if (pending === token) { pending = null; pendingKey = null }
      refreshExpiry()
      render()
    }
  }
  const approve = async () => {
    const prompt = current()
    if (!open || !prompt || pending || !deps.connected()) return
    const typed = value
    // Another terminal may have opened the owner's remember window. Ordinary
    // approvals always let the kernel check it; new authority needs fresh proof.
    if (!typed && prompt.kind !== "critical_approval") {
      error = PASSKEY_REFUSALS.PASSKEY_REQUIRED!
      render()
      return
    }
    const minutes = prompt.kind === "critical_approval" ? remember : 0
    value = ""
    await send(prompt, prompt.approve_choice_id,
      typed ? { passkey: typed, rememberMinutes: minutes || null, ...(["access_grant", "access_extension", "sudo"].includes(prompt.kind) ? { accessLifetime: accessLifetime ?? startingLifetime(prompt) } : {}) } : undefined)
  }
  const refuse = async () => {
    const prompt = current()
    if (!open || !prompt || pending || !deps.connected()) return
    value = ""
    await send(prompt, prompt.refuse_choice_id)
  }
  const cycleRemember = () => {
    if (!open || pending) return
    const prompt = current()
    if (prompt && prompt.kind !== "critical_approval") {
      const max = prompt.max_lifetime_minutes
      const start = startingLifetime(prompt)
      if (!max || start === null) return
      // Sudo windows: one hour by default, 2, 4 or 8 hours per fresh passkey.
      const minutes = prompt.kind === "sudo" ? SUDO_WINDOW_MINUTES : [typeof start === "number" ? start : 0, 5, 15, 30, 60, 120, max]
      const options: AccessLifetime[] = [...new Set(minutes)].filter(m => m > 0 && m <= max).sort((a, b) => a - b)
      // Only a new access grant can last until its agent exits; an extension adds time.
      if (prompt.kind === "access_grant") options.push(UNTIL_AGENT_EXITS)
      accessLifetime = options[(options.indexOf(accessLifetime ?? start) + 1) % options.length]!
      render()
      return
    }
    const options: readonly number[] = PASSKEY_REMEMBER_MINUTES
    remember = options[(options.indexOf(remember) + 1) % options.length]!
    render()
  }
  const toggleDetails = () => { if (current()?.sudo_request) { detailsOpen = !detailsOpen; render() } }
  const hide = () => {
    hidden = true
    syncOpen()
    render()
  }
  return {
    view, approve, refuse, cycleRemember, hide, toggleDetails,
    invalidateConnectionState(options: { readonly preservePrompts?: boolean } = {}) {
      connectionEpoch++
      pending = null
      pendingKey = null
      deps.serverClock?.invalidate()
      clockRead = false
      if (!options.preservePrompts) {
        prompts = []
        answered.clear()
        awaitingClosure.clear()
        authoritativeFeed = false
      }
      hidden = true
      clearEntry()
      syncOpen()
      refreshExpiry()
      render()
    },
    isOpen: () => open,
    ownsInput: () => open || claimedInputTurn,
    /** The kernel's current set, from `passkey_prompts_changed`. A prompt
     * that arrives shows an indicator; F8 or a click explicitly opens it. */
    apply(next: PasskeyPrompt[], metadata?: {kernel_now_ms?: number; closed?: PasskeyPromptClosure[]}) {
      if (typeof metadata?.kernel_now_ms === "number" && Number.isFinite(metadata.kernel_now_ms) && metadata.kernel_now_ms >= 0) {
        authoritativeFeed = true
        deps.serverClock?.observe?.(metadata.kernel_now_ms)
        for (const closed of metadata.closed ?? []) {
          const key = promptKey(closed)
          if (awaitingClosure.get(key) === closed.kind && closed.reason === "expired") deps.notify(approvalExpiredMessage(closed.kind))
          awaitingClosure.delete(key)
        }
        for (const prompt of next) awaitingClosure.set(promptKey(prompt), prompt.kind)
        while (awaitingClosure.size > 128) awaitingClosure.delete(awaitingClosure.keys().next().value!)
      }
      // Inspect the old set before a timeout-removal feed drops it.
      refreshExpiry()
      const shown = current()
      const listed = new Set(next.map(promptKey))
      for (const key of answered) if (!listed.has(key)) answered.delete(key)
      prompts = next.filter((prompt) => !answered.has(promptKey(prompt)))
      reselect(shown)
      syncOpen()
      refreshExpiry()
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
      if (event.ctrl && event.name === "d") { toggleDetails(); return true }
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
      if (expiryTimer) clearTimeout(expiryTimer)
      deps.serverClock?.invalidate()
      clearEntry()
      if (open) { open = false; deps.onClose() }
    },
  }
}
