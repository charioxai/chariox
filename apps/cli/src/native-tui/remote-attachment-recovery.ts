import { LocalIpcError } from "@chariox/kernel-client/ipc"

// RR-07: a native-TUI attachment carried through the gateway is swept by the target
// after ~30s without a heartbeat (a relay outage stops the origin's heartbeats). The
// next attachment-bearing request then returns `attachment_not_found`. Attach mints a
// fresh id, so the client re-attaches, adopts the new id and re-subscribes. A revoked
// grant or a gone session is surfaced, never resurrected.

export type RemoteAttachmentRef = { id: string }

export type RemoteAttachmentRecoveryOutcome =
  // Re-attached: the holder carries a fresh id and the subscription was renewed.
  | "recovered"
  // The link is still down, or recovery is not yet complete: keep the current attachment
  // and try again later. A state-changing request is never silently retried on this.
  | "transient"
  // Session gone / grant refused / client detached: surface it and stop. No resurrection.
  | "fatal"

const ATTACHMENT_NOT_FOUND = "attachment_not_found"

type ErrorKind = "attachment" | "fatal" | "transient"

// Classify by the kernel's typed error code, never by prose. Only the exact
// `attachment_not_found` authorizes a re-attach; any other non-retryable refusal
// (unauthorized, session_not_found, access revoked, …) is fatal; a retryable transport
// error is transient so the pump waits out the outage without retrying a mutation.
function classify(error: unknown): ErrorKind {
  if (!(error instanceof LocalIpcError)) return "transient"
  if (error.code === ATTACHMENT_NOT_FOUND) return "attachment"
  return error.retryable ? "transient" : "fatal"
}

export type RemoteAttachmentRecoveryOptions = {
  attachment: RemoteAttachmentRef
  reattach: () => Promise<string>
  resubscribe: (attachmentId: string) => Promise<void>
  isActive: () => boolean
  detach: (attachmentId: string) => Promise<void>
  onRecovered?: (attachmentId: string) => void
  onFatal?: (error: unknown) => void
}

// Serialises concurrent lapses (the pump and a keystroke can both observe one) behind a
// single in-flight re-attach.
export function createRemoteAttachmentRecovery(
  options: RemoteAttachmentRecoveryOptions,
): { recover: (error: unknown) => Promise<RemoteAttachmentRecoveryOutcome> } {
  let inFlight: Promise<RemoteAttachmentRecoveryOutcome> | null = null

  const fatal = (error: unknown): RemoteAttachmentRecoveryOutcome => {
    options.onFatal?.(error)
    return "fatal"
  }

  const run = async (error: unknown): Promise<RemoteAttachmentRecoveryOutcome> => {
    const kind = classify(error)
    if (kind === "transient") return "transient"
    if (kind === "fatal") return fatal(error)
    if (!options.isActive()) return "fatal"

    let nextAttachmentId: string
    try {
      nextAttachmentId = await options.reattach()
    } catch (reattachError) {
      const reattachKind = classify(reattachError)
      // The link is still flaky: keep the old attachment and let the caller retry.
      if (reattachKind === "transient") return "transient"
      // Re-attach refused (session gone / grant revoked): surface it.
      return fatal(reattachError)
    }

    if (!options.isActive()) {
      // Torn down (user exit / explicit detach) while re-attaching: release the new
      // attachment so it does not leak on the target.
      await options.detach(nextAttachmentId).catch(() => {})
      return "fatal"
    }

    options.attachment.id = nextAttachmentId
    try {
      await options.resubscribe(nextAttachmentId)
    } catch {
      // The id is adopted (the pump already reads it), but the event subscription is not
      // live yet: report "transient" so recovery runs again rather than hiding it.
      return "transient"
    }
    options.onRecovered?.(nextAttachmentId)
    return "recovered"
  }

  const recover = (error: unknown): Promise<RemoteAttachmentRecoveryOutcome> => {
    if (inFlight) return inFlight
    const current = run(error).finally(() => {
      if (inFlight === current) inFlight = null
    })
    inFlight = current
    return current
  }

  return { recover }
}
