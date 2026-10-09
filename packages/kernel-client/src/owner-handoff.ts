/** MP-08 / MP-10 / MP-11 A07 (protocol 477): protected owner hand-off.
 * The interaction carries only safe target metadata. Owner input goes in
 * `RespondToHandoff`, straight to the bound browser field; never as an
 * interaction reply. */
import type { RuntimeInteraction } from "./kernel-types-runtime.js"

export type HandoffKind = "click" | "code" | "secret"
export type HandoffReason = "model_refusal" | "automation_disallowed" | "human_verification" | "owner_authorization"
export type HandoffChangeLine = { op: "keep" | "add" | "remove"; text: string }

export type HandoffTarget = {
  tab_id: string
  generation: number
  document_id: string
  node_ref: string
  origin: string
  path: string
  label: string
}

export type RuntimeHandoff = {
  kind: HandoffKind
  reason: HandoffReason
  agent_id: string
  task_id: string
  obligation_id: string
  explanation: string
  target: HandoffTarget
  change?: HandoffChangeLine[]
  expires_at_ms: number
  save_to_vault_offered?: boolean
}

export type HandoffResponseAction =
  | { kind: "click" }
  | { kind: "enter_value"; value: string; save_to_vault_key?: string }
  | { kind: "done" }
  | { kind: "cancel" }

export type HandoffStatus = "completed" | "failed" | "cancelled" | "expired" | "uncertain"

export type HandoffOutcome = {
  handoff_id: string
  status: HandoffStatus
  action: string
  reason_code?: string | null
  saved_to_vault?: boolean
}

export type HandoffResolvedResponse = { HandoffResolved: { outcome: HandoffOutcome } }

export function respondToHandoffRequest(sessionId: string, interactionId: string, action: HandoffResponseAction) {
  return { RespondToHandoff: { session_id: sessionId, interaction_id: interactionId, action } }
}

export function interactionHandoff(interaction: RuntimeInteraction): RuntimeHandoff | null {
  return interaction.handoff ?? null
}

export function handoffActionLabel(handoff: RuntimeHandoff): string {
  if (handoff.kind === "click") return `Click “${handoff.target.label}”`
  return handoff.kind === "code" ? `Enter code in “${handoff.target.label}”` : `Enter secret in “${handoff.target.label}”`
}

export function handoffReasonLabel(reason: HandoffReason): string {
  return {
    model_refusal: "The agent's model refused this step",
    automation_disallowed: "This site disallows automation",
    human_verification: "A human verification is required",
    owner_authorization: "Your authorization is required",
  }[reason]
}

/** The intended change as diff lines: `+` add, `-` remove, ` ` keep. */
export function handoffChangeLines(handoff: RuntimeHandoff): string[] {
  return (handoff.change ?? []).map(line => `${line.op === "add" ? "+" : line.op === "remove" ? "-" : " "} ${line.text}`)
}

export function handoffOutcomeText(outcome: HandoffOutcome): string {
  const reason = outcome.reason_code ? ` (${outcome.reason_code.replaceAll("_", " ")})` : ""
  const saved = outcome.saved_to_vault ? "; saved to Vault" : ""
  return `Hand-off ${outcome.status}${reason}${saved}`
}
