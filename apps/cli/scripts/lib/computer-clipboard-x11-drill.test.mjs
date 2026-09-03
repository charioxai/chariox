import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"

import {
  assertRetainedClipboardEvidenceIsRedacted,
  clipboardCaseSummary,
  clipboardInterruptionWindowMs,
  redactClipboardValue,
} from "./computer-clipboard-x11-drill.mjs"

test("clipboard drill redaction never expands an empty value", () => {
  assert.equal(redactClipboardValue("before clipboard after", "clipboard"), "before [redacted clipboard] after")
  assert.equal(redactClipboardValue("unchanged", ""), "unchanged")
})

test("clipboard drill retains only digest and bounded size metadata", () => {
  const value = "Clipboard Grüße 世界\nsecond line\n"
  assert.deepEqual(clipboardCaseSummary("unicode-newlines", value), {
    name: "unicode-newlines",
    utf8ByteCount: Buffer.byteLength(value, "utf8"),
    characterCount: [...value].length,
    sha256: createHash("sha256").update(value).digest("hex"),
  })
})

test("clipboard drill fails closed when retained evidence contains clipboard text", () => {
  const value = "clipboard-canary-value"
  assert.doesNotThrow(() =>
    assertRetainedClipboardEvidenceIsRedacted(
      { report: "[redacted clipboard]", output: "safe" },
      value,
    ),
  )
  assert.throws(
    () => assertRetainedClipboardEvidenceIsRedacted({ report: `leak=${value}` }, value),
    /retained clipboard evidence contains clipboard text/,
  )

  const multiline = "clipboard-first-line\nclipboard-second-line\n"
  assert.throws(
    () => assertRetainedClipboardEvidenceIsRedacted({ report: multiline }, multiline),
    /retained clipboard evidence contains clipboard text/,
  )
  assert.throws(
    () =>
      assertRetainedClipboardEvidenceIsRedacted(
        JSON.stringify({ report: multiline }),
        multiline,
      ),
    /retained clipboard evidence contains clipboard text/,
  )
})

test("clipboard drill interruption window is explicit and bounded", () => {
  assert.equal(clipboardInterruptionWindowMs(undefined), 0)
  assert.equal(clipboardInterruptionWindowMs("2500"), 2500)
  for (const value of ["", "-1", "1.5", "60001", "not-a-number"]) {
    assert.throws(() => clipboardInterruptionWindowMs(value), /integer between 0 and 60000/)
  }
})
