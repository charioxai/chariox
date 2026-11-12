import { LocalIpcError } from "@chariox/kernel-client/ipc"

// RR-07: a native-TUI attachment carried through the gateway is swept by the target
// after ~30s without a heartbeat (a relay outage stops the origin's heartbeats). The
// next attachment-bearing request then returns `attachment_not_found`. Attach mints a
// fresh id, so the client re-attaches, adopts the new id and renews its subscription.
// Existing display content stays in place while new output resumes. A revoked grant or a gone session is
// surfaced, never resurrected.

export type RemoteAttachmentRef = { id: string }

export type RemoteAttachmentRecoveryOutcome =
  // Re-attached: the holder carries a fresh id. The event subscription is live, or is
  // pending and will be retried on the next pump tick (never silently left dead).
  | "recovered"
  // The link is still down: keep the current attachment and try again later. A
  // state-changing request is never silently retried on this outcome.
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

export type RemoteAttachmentRecovery = {
  // Re-attaches on a definite attachment_not_found. Serialised: concurrent lapses (the
  // pump and a keystroke) share one in-flight re-attach.
  recover: (error: unknown) => Promise<RemoteAttachmentRecoveryOutcome>
  // Retries a deferred re-subscription. The pump calls it every tick so a subscription
  // that could not be renewed during recovery is re-established even while polls succeed.
  settlePendingSubscription: () => Promise<void>
}

export function createRemoteAttachmentRecovery(
  options: RemoteAttachmentRecoveryOptions,
): RemoteAttachmentRecovery {
  let inFlight: Promise<RemoteAttachmentRecoveryOutcome> | null = null
  let pendingSubscriptionId: string | null = null
  let settling = false
  let refused = false

  const fatal = (error: unknown): RemoteAttachmentRecoveryOutcome => {
    refused = true
    options.onFatal?.(error)
    return "fatal"
  }

  const settlePendingSubscription = async (): Promise<void> => {
    if (settling) return
    const id = pendingSubscriptionId
    if (id === null) return
    settling = true
    try {
      if (!options.isActive()) {
        pendingSubscriptionId = null
        return
      }
      await options.resubscribe(id)
      // A newer recovery may have superseded this id while we awaited.
      if (pendingSubscriptionId === id) {
        pendingSubscriptionId = null
        options.onRecovered?.(id)
      }
    } catch (error) {
      if (classify(error) === "fatal") {
        pendingSubscriptionId = null
        fatal(error)
        return
      }
      // Keep it pending; the next pump tick retries it.
    } finally {
      settling = false
    }
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
      // The link is still flaky: keep the old attachment and let the caller retry.
      if (classify(reattachError) === "transient") return "transient"
      // Re-attach refused (session gone / grant revoked): surface it.
      return fatal(reattachError)
    }

    if (!options.isActive()) {
      // Torn down (user exit / explicit detach) while re-attaching: release the new
      // attachment so it does not leak on the target.
      await options.detach(nextAttachmentId).catch(() => {})
      return "fatal"
    }

    // Adopt the id (the pump reads it immediately for the screen catch-up) and renew the
    // subscription. If the renewal fails it is tracked as pending, not hidden: the pump's
    // next settle retries it.
    options.attachment.id = nextAttachmentId
    pendingSubscriptionId = nextAttachmentId
    await settlePendingSubscription()
    return refused ? "fatal" : "recovered"
  }

  const recover = (error: unknown): Promise<RemoteAttachmentRecoveryOutcome> => {
    if (refused) return Promise.resolve("fatal")
    if (inFlight) return inFlight
    const current = run(error).finally(() => {
      if (inFlight === current) inFlight = null
    })
    inFlight = current
    return current
  }

  return { recover, settlePendingSubscription }
}

// The factory allocates a new command identity for a definitively refused request.
// A timeout has an unknown outcome and must reach the caller without another send.
export async function sendWithRecoveredAttachment<T>(
  send: () => Promise<T>,
  recovery?: RemoteAttachmentRecovery,
): Promise<T> {
  try {
    return await send()
  } catch (error) {
    if (error instanceof LocalIpcError && error.code === ATTACHMENT_NOT_FOUND
      && recovery && await recovery.recover(error) === "recovered") return await send()
    throw error
  }
}
