import assert from "node:assert/strict"
import { createHash } from "node:crypto"

export function redactClipboardValue(value, clipboardText) {
  if (clipboardText.length === 0) return value
  return value.replaceAll(clipboardText, "[redacted clipboard]")
}

export function clipboardCaseSummary(name, value) {
  return textCaseSummary(name, value)
}

export function textCaseSummary(name, value) {
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
  assertRetainedTextIsRedacted(
    evidence,
    clipboardText,
    "retained clipboard evidence contains clipboard text",
  )
}

export function assertRetainedTextIsRedacted(evidence, text, message) {
  if (text.length === 0) return
  const escapedText = JSON.stringify(text).slice(1, -1)
  assert.equal(
    containsText(evidence, text, escapedText, new WeakSet()),
    false,
    message,
  )
}

function containsText(value, text, escapedText, seen) {
  if (typeof value === "string") {
    return (
      value.includes(text) ||
      (escapedText !== text && value.includes(escapedText))
    )
  }
  if (!value || typeof value !== "object") return false
  if (seen.has(value)) return false
  seen.add(value)
  if (Array.isArray(value)) {
    return value.some((entry) =>
      containsText(entry, text, escapedText, seen),
    )
  }
  return Object.entries(value).some(
    ([key, entry]) =>
      containsText(key, text, escapedText, seen) ||
      containsText(entry, text, escapedText, seen),
  )
}
