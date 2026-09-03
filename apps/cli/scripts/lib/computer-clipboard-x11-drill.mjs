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

export function utf8TextFromChunks(chunks) {
  return Buffer.concat(chunks).toString("utf8")
}

export function assertRetainedClipboardEvidenceIsRedacted(evidence, clipboardText) {
  if (clipboardText.length === 0) return
  const escapedClipboardText = JSON.stringify(clipboardText).slice(1, -1)
  assert.equal(
    containsClipboardText(evidence, clipboardText, escapedClipboardText, new WeakSet()),
    false,
    "retained clipboard evidence contains clipboard text",
  )
}

function containsClipboardText(value, clipboardText, escapedClipboardText, seen) {
  if (typeof value === "string") {
    return (
      value.includes(clipboardText) ||
      (escapedClipboardText !== clipboardText && value.includes(escapedClipboardText))
    )
  }
  if (!value || typeof value !== "object") return false
  if (seen.has(value)) return false
  seen.add(value)
  if (Array.isArray(value)) {
    return value.some((entry) =>
      containsClipboardText(entry, clipboardText, escapedClipboardText, seen),
    )
  }
  return Object.entries(value).some(
    ([key, entry]) =>
      containsClipboardText(key, clipboardText, escapedClipboardText, seen) ||
      containsClipboardText(entry, clipboardText, escapedClipboardText, seen),
  )
}
