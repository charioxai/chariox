import assert from "node:assert/strict"
import test from "node:test"
import { renderPatchFilesDiffHtml } from "./diff-render.js"

// A change with no text to diff (a binary write) says what it is: the body is its note, never empty.
test("a binary write with no diff draws its note and size", () => {
  const html = renderPatchFilesDiffHtml([{ kind: "update", filePath: "logo.png", title: "Wrote logo.png", diff: null, note: "Binary file · 12 KB" }])
  assert.match(html, /Binary file · 12 KB/)
  assert.doesNotMatch(html, /no textual changes/)
})

test("a move keeps its title and a delete still says the file was deleted", () => {
  const moved = renderPatchFilesDiffHtml([{ kind: "move", filePath: "a.txt", title: "Moved a.txt -> b.txt", diff: null, note: "Moved to b.txt" }])
  assert.match(moved, /Moved a\.txt -&#62; b\.txt/)
  assert.equal(moved.match(/Moved to b\.txt/g)?.length ?? 0, 0, "the title already says it")
  assert.match(renderPatchFilesDiffHtml([{ kind: "delete", filePath: "old.txt", title: "Deleted old.txt", diff: null }]), /file deleted/)
})
