import { LocalIpcError } from "@chariox/kernel-client/ipc"

import { describeCliError } from "./runtime.js"

export function isSessionUnavailableError(error: unknown): boolean {
  const message = describeCliError(error)
  return /session `[^`]+` was not found/i.test(message)
    || /attachment `[^`]+` was not found/i.test(message)
    || /does not belong to session/i.test(message)
    || /cannot perform `[^`]+` while ended/i.test(message)
}

// RR-07: after a >30s relay outage the origin stops heartbeating and the kernel sweeps
// the attachment. The next attachment-bearing request then refuses with the typed
// `attachment_not_found`. That is a swept attachment the client re-attaches the same
// Room for — distinct from a gone session or a revoked grant, which are surfaced and
// never resurrected. Classify by the kernel's typed code, never by prose: the swept
// attachment and a genuine session loss can read the same ("attachment `...` was not
// found"), so `isSessionUnavailableError` alone would tear a recoverable Room down.
export function isSweptAttachmentError(error: unknown): boolean {
  return error instanceof LocalIpcError && error.code === "attachment_not_found"
}
