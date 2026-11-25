/** What a Room's machine approval became after its card left: the kernel keeps one receipt per approval. */
export type MachineApprovalReceipt = {
  id: string
  shown_in_agent_id: string | null
  message: string
  state: "approving" | "sent" | "expired"
  expires_at_ms: number | null
}

const EXPIRED_VISIBLE_MS = 10 * 60 * 1000

export function machineApprovalReceipts(value: unknown): MachineApprovalReceipt[] {
  if (!Array.isArray(value)) return []
  return value.filter((receipt): receipt is MachineApprovalReceipt =>
    typeof receipt === "object" && receipt !== null
    && typeof (receipt as MachineApprovalReceipt).id === "string"
    && typeof (receipt as MachineApprovalReceipt).message === "string"
    && ["approving", "sent", "expired"].includes((receipt as MachineApprovalReceipt).state))
}

/** One line per receipt that still needs the person's eye. Approving stays until the service answers; Sent is the
 * stripe closing; Expired stays for ten minutes so a missed approval is not silent. */
export function machineApprovalLinesForAgent(
  receipts: readonly MachineApprovalReceipt[] | undefined,
  agentId: string | null | undefined,
  now = Date.now(),
): string[] {
  if (!agentId) return []
  return (receipts ?? []).flatMap(receipt => {
    if (receipt.shown_in_agent_id !== agentId) return []
    if (receipt.state === "approving") return [`${receipt.message} — Approving…`]
    if (receipt.state === "expired" && receipt.expires_at_ms !== null && now - receipt.expires_at_ms < EXPIRED_VISIBLE_MS) {
      return [`${receipt.message} — Expired at ${new Date(receipt.expires_at_ms).toISOString().replace(/\.\d+Z$/, "Z")}`]
    }
    return []
  })
}

/** The session after the kernel's interactions event: cards and receipts replace each other's earlier values together. */
export function withRuntimeInteractions<TSession extends { active_interactions?: unknown[]; machine_approval_receipts?: MachineApprovalReceipt[] }>(
  session: TSession,
  activeInteractions: unknown[],
  receipts: unknown,
): TSession {
  return { ...session, active_interactions: activeInteractions, machine_approval_receipts: machineApprovalReceipts(receipts) }
}
