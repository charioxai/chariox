import assert from "node:assert/strict"
import test from "node:test"
import { notesRequest, noteOverlayBox, notePromptDraft, notesMinimumProtocolVersion } from "./notes.js"

test("MD-N3 / MP-08: reject old/unknown kernels before notes requests", () => {
  for (const version of [417, 423, NaN, Infinity, 443.5]) assert.throws(() => notesRequest({ op: "ask", note_id: "n" }, version))
  assert.equal(notesMinimumProtocolVersion, 443)
  assert.deepEqual(notesRequest({ op: "list", window: { kind: "panel", window_id: "transcript" } }, 443), { Notes: { command: { op: "list", window: { kind: "panel", window_id: "transcript" } } } })
})
test("MD-N2 / MP-08: overlay maps page coordinates, clips and rejects invalid geometry", () => {
  assert.deepEqual(noteOverlayBox({ x: -10, y: 40, width: 30, height: 20 }, { width: 1280, height: 800 }, { x: 200, y: 100, width: 640, height: 400 }), { x: 200, y: 120, width: 10, height: 10 })
  assert.equal(noteOverlayBox({ x: 0, y: 0, width: NaN, height: 1 }, { width: 1, height: 1 }, { x: 0, y: 0, width: 1, height: 1 }), null)
})
test("MD-N3 / MP-10: Ask leaves a visibly marked draft with existing attachment fields", () => {
  const attachment = { url: "chariox-terminal://prompt-attachment/n/note.txt", mime: "text/plain", filename: "Chariox note.txt", contents_base64: "Zml4dHVyZQ==" }
  const draft = notePromptDraft({ event: "prompt_draft", note_id: "n", text: "[Chariox note n]\nSelected text:\nfixture\nComment:\nexplain\n[/Chariox note]", attachment })
  assert.deepEqual(draft.attachments, [attachment])
  assert.ok(draft.text.includes("Selected text:"))
  assert.throws(() => notePromptDraft({ event: "selection_changed", selection: null }))
})
