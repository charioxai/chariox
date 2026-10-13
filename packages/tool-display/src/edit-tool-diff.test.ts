import assert from "node:assert/strict"
import test from "node:test"

import { diffView, parseUnifiedDiff } from "./diff-view.js"
import { formatToolDisplay, formatToolTranscriptUpdate, readApplyPatchFiles } from "./index.js"

// MP-08 / MP-10 (PANE-09, R16): edit tools render as diffs. The OpenCode
// record is the real completed `edit` update from a live drill (kernel history).
const opencodeEdit = {
  id: "prt_131b053b10017FpPRS7rBcYIjy", tool: "edit", status: "completed", title: "README.md",
  input: { filePath: "/w/README.md", newString: "before: updated foc2", oldString: "before: updated fcx2" },
  output: "Edit applied successfully.",
}

const rows = (diff: string | null) => parseUnifiedDiff(diff ?? "").flatMap((file) => file.rows)
  .map((row) => row.kind === "line" ? `${row.line[0]}${row.text}` : row.kind)

test("an OpenCode edit becomes a patch block with removed and added lines", () => {
  const [file] = readApplyPatchFiles(opencodeEdit)
  assert.equal(file!.kind, "update")
  assert.equal(file!.filePath, "/w/README.md")
  assert.deepEqual(rows(file!.diff), ["gap", "rbefore: updated fcx2", "abefore: updated foc2"].slice(1))
  assert.equal(formatToolDisplay(opencodeEdit).blocks[0]!.kind, "patch")
  const view = diffView(parseUnifiedDiff(file!.diff!))
  assert.equal(`${view.verb} ${view.target} +${view.additions} -${view.deletions}`, "Edited /w/README.md +1 -1")
})

test("Claude Edit keeps shared lines as context; MultiEdit makes one hunk per edit", () => {
  const [edit] = readApplyPatchFiles({ id: "t", tool: "Edit", status: "completed", input: { file_path: "/w/a.py", old_string: "def f():\n    return 1\n", new_string: "def f():\n    return 2\n" } })
  assert.deepEqual(rows(edit!.diff), [" def f():", "r    return 1", "a    return 2"].map((row) => row.replace(/^ /, "c")))
  const [multi] = readApplyPatchFiles({ id: "t", tool: "MultiEdit", status: "completed", input: { file_path: "/w/b.txt", edits: [{ old_string: "a", new_string: "b" }, { old_string: "x\n", new_string: "" }] } })
  assert.deepEqual(rows(multi!.diff), ["ra", "ab", "gap", "rx"])
})

test("write tools show content without claiming the file was created", () => {
  const [file] = readApplyPatchFiles({ id: "t", tool: "write", status: "completed", input: { filePath: "/w/new.md", content: "# t\n\nbody\n" } })
  assert.equal(file!.kind, "update")
  assert.deepEqual(rows(file!.diff), ["a# t", "a", "abody"])
  assert.equal(diffView(parseUnifiedDiff(file!.diff!)).verb, "Edited")
})

test("edit tools without a path or strings are not patches", () => {
  assert.deepEqual(readApplyPatchFiles({ id: "t", tool: "edit", status: "pending" }), [])
  assert.deepEqual(readApplyPatchFiles({ id: "t", tool: "Edit", input: { file_path: "/w/a" } }), [])
})

// MP-08 / MP-10: attempted edits must keep their status and diagnostics.
test("unapplied edit/write calls keep generic status and error/output", () => {
  for (const tool of ["edit", "Edit", "MultiEdit", "write", "Write"]) {
    for (const status of ["error", "cancelled", "denied", "pending", "running", undefined]) {
      const update = {
        id: "rejected", tool, ...(status ? { status } : {}),
        input: { filePath: "/w/a.py", oldString: "old", newString: "new", content: "new", edits: [{ old_string: "old", new_string: "new" }] },
        error: "String to replace not found", output: "User rejected the edit",
      }
      assert.deepEqual(readApplyPatchFiles(update), [], `${tool}: ${status}`)
      const display = formatToolDisplay(update)
      assert.ok(display.blocks.every(block => block.kind !== "patch"))
      assert.match(formatToolTranscriptUpdate(update), /String to replace not found/)
      assert.match(formatToolTranscriptUpdate(update), /User rejected the edit/)
      assert.match(formatToolTranscriptUpdate(update), /String to replace not found/)
      if (status) assert.ok(formatToolTranscriptUpdate(update).includes(status.toUpperCase()))
    }
  }
})
