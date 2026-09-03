import assert from "node:assert/strict"
import { createHash } from "node:crypto"

export function redactClipboardValue(value, clipboardText) {
  if (clipboardText.length === 0) return value
  return value.replaceAll(clipboardText, "[redacted clipboard]")
}

export function clipboardCaseSummary(name, value) {
  return {
    name,
    utf8ByteCount: Buffer.byteLength(value, "utf8"),
    characterCount: [...value].length,
    sha256: createHash("sha256").update(value).digest("hex"),
  }
}

export function clipboardInterruptionWindowMs(value) {
  if (value === undefined) return 0
  if (!/^\d+$/.test(value)) {
    throw new Error("clipboard interruption window must be an integer between 0 and 60000")
  }
  const milliseconds = Number(value)
  if (!Number.isSafeInteger(milliseconds) || milliseconds > 60_000) {
    throw new Error("clipboard interruption window must be an integer between 0 and 60000")
  }
  return milliseconds
}

export function assertRetainedClipboardEvidenceIsRedacted(evidence, clipboardText) {
  if (clipboardText.length === 0) return
  assert.equal(
    JSON.stringify(evidence).includes(clipboardText),
    false,
    "retained clipboard evidence contains clipboard text",
  )
}
