import type { HandoffOutcome, HandoffResponseAction, RuntimeHandoff } from "@chariox/kernel-client/owner-handoff"
import { handoffActionLabel, handoffOutcomeText } from "@chariox/kernel-client/owner-handoff"
import type { RuntimeInteraction, RuntimeInteractionChoice } from "./cli-types.js"

/** MP-08 / MP-10 / MP-11 A07: protected owner entry for a hand-off in the
 * approval panel. The typed value lives only here until it is sent with
 * `RespondToHandoff`, then it is dropped; views expose only its length. */
const VALUE_MAX_LENGTH = 4096
const CONTROL_CHARACTERS = /[\u0000-\u001f\u007f-\u009f]/
export const HANDOFF_CLICK_CHOICE = "handoff:click"
export const HANDOFF_ENTER_CHOICE = "handoff:enter"

export type HandoffEntryView = { kind: "code" | "secret"; length: number; saveOffered: boolean; saveToVault: boolean }

export function interactionHandoff(interaction: RuntimeInteraction | null): RuntimeHandoff | null {
  return (interaction as { handoff?: RuntimeHandoff | null } | null)?.handoff ?? null
}

/** The panel's choices: the protected owner action first, then the kernel's
 * safe choices (Done in browser, Cancel). */
export function handoffChoices(interaction: RuntimeInteraction): RuntimeInteractionChoice[] {
  const handoff = interactionHandoff(interaction)
  if (!handoff) return interaction.choices
  const protectedChoice: RuntimeInteractionChoice = handoff.kind === "click"
    ? { id: HANDOFF_CLICK_CHOICE, label: `${handoffActionLabel(handoff)} as you`, reply: "" }
    : { id: HANDOFF_ENTER_CHOICE, label: `${handoffActionLabel(handoff)}…`, reply: "" }
  return [protectedChoice, ...interaction.choices]
}

/** A Vault key derived from the target host; the owner opts in with Tab. */
export function handoffVaultKey(handoff: RuntimeHandoff): string {
  const host = (() => { try { return new URL(handoff.target.origin).hostname } catch { return "site" } })()
  return `${host.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "")}-handoff`
}

export function createHandoffEntry(handoff: RuntimeHandoff) {
  let value = ""
  let saveToVault = false
  return {
    view: (): HandoffEntryView => ({
      kind: handoff.kind === "code" ? "code" : "secret", length: Array.from(value).length,
      saveOffered: handoff.save_to_vault_offered === true, saveToVault,
    }),
    /** Typed text exactly as sent; refused whole when it holds control characters. */
    add(text: string): boolean {
      if (!text || CONTROL_CHARACTERS.test(text) || Array.from(value + text).length > VALUE_MAX_LENGTH) return false
      value += text
      return true
    },
    backspace() { value = Array.from(value).slice(0, -1).join("") },
    toggleSave() { if (handoff.save_to_vault_offered) saveToVault = !saveToVault },
    /** The one action to send; the entry is cleared either way. */
    take(): HandoffResponseAction | null {
      const typed = value
      value = ""
      if (!typed) return null
      return saveToVault ? { kind: "enter_value", value: typed, save_to_vault_key: handoffVaultKey(handoff) } : { kind: "enter_value", value: typed }
    },
    clear() { value = "" },
  }
}

export function handoffKeyText(event: { name: string; sequence?: string; shift?: boolean }): string {
  if (event.sequence && !CONTROL_CHARACTERS.test(event.sequence)) return event.sequence
  if (event.name === "space") return " "
  if (Array.from(event.name).length !== 1 || CONTROL_CHARACTERS.test(event.name)) return ""
  return event.shift ? event.name.toLocaleUpperCase() : event.name
}

export function handoffResultText(outcome: HandoffOutcome | null, error: unknown): string {
  if (outcome) return handoffOutcomeText(outcome)
  const text = error instanceof Error ? error.message : String(error ?? "")
  return text.includes("PASSKEY_ALREADY_ANSWERED") || text.includes("already answered")
    ? "This hand-off was already answered."
    : "The kernel did not confirm this hand-off answer. Check the agent before retrying."
}
