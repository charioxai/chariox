import assert from "node:assert/strict"
import test from "node:test"

import { formatToolDisplay, readApplyPatchFiles, type ToolDisplay, type ToolTranscriptUpdate } from "./index.js"

// A "Wrote a file" row that opens to nothing must never happen (owner, Landing 7): whatever event shape a provider gives a
// file write or edit, the opened row shows the file's path and its change (a diff, a preview of a new file, or the size of a
// binary one).
const page = "<!doctype html>\n<html>\n<body>\n<p>Monthly active users</p>\n</body>\n</html>\n"
const completed = (tool: string, input: unknown, extra: Partial<ToolTranscriptUpdate> = {}): ToolTranscriptUpdate => ({ id: "call-1", tool, status: "completed", input, ...extra })
const running = (tool: string, input: unknown): ToolTranscriptUpdate => ({ id: "call-1", tool, status: "running", input })

const shapes: Record<string, { update: ToolTranscriptUpdate; path: string; shows: RegExp }> = {
  "Codex apply_patch, new file with its content": {
    update: { id: "c", tool: "apply_patch", status: "completed", title: "1 file changes", raw: JSON.stringify([{ kind: { type: "add" }, path: "/w/chart.html", diff: page }], null, 2) },
    path: "/w/chart.html", shows: /Monthly active users/,
  },
  "Codex apply_patch, the stored preview without the content": {
    update: { id: "c", tool: "apply_patch", status: "completed", title: "1 file changes", raw: JSON.stringify([{ kind: { type: "add" }, path: "/w/chart.html" }]) },
    path: "/w/chart.html", shows: /./,
  },
  "Codex apply_patch, update with a unified diff": {
    update: { id: "c", tool: "apply_patch", status: "completed", raw: JSON.stringify([{ kind: { type: "update" }, path: "/w/a.ts", diff: "@@ -1 +1 @@\n-old\n+new\n" }]) },
    path: "/w/a.ts", shows: /new/,
  },
  "Claude Write": { update: completed("Write", { file_path: "/w/chart.html", content: page }), path: "/w/chart.html", shows: /Monthly active users/ },
  "Claude Write, still running": { update: running("Write", { file_path: "/w/chart.html", content: page }), path: "/w/chart.html", shows: /Monthly active users/ },
  "Claude Write that failed": { update: completed("Write", { file_path: "/w/chart.html", content: page }, { status: "error", error: "EACCES: permission denied" }), path: "/w/chart.html", shows: /permission denied/ },
  "Claude Edit": { update: completed("Edit", { file_path: "/w/a.ts", old_string: "old", new_string: "new" }), path: "/w/a.ts", shows: /new/ },
  "Claude MultiEdit": { update: completed("MultiEdit", { file_path: "/w/a.ts", edits: [{ old_string: "a", new_string: "b" }] }), path: "/w/a.ts", shows: /b/ },
  "OpenCode write": { update: completed("write", { filePath: "/w/chart.html", content: page }), path: "/w/chart.html", shows: /Monthly active users/ },
  "OpenCode edit": { update: completed("edit", { filePath: "/w/a.ts", oldString: "old", newString: "new" }), path: "/w/a.ts", shows: /new/ },
  "OpenCode apply_patch": { update: completed("apply_patch", { patchText: "*** Begin Patch\n*** Add File: /w/n.txt\n+hello\n*** End Patch" }), path: "/w/n.txt", shows: /hello/ },
  "Chariox write_artifact (text), running": { update: running("chariox.write_artifact", { path: "monthly_active_users.html", domain: "text", content_text: page }), path: "monthly_active_users.html", shows: /Monthly active users/ },
  "Chariox write_artifact (text), completed with only its input": { update: completed("chariox.write_artifact", { path: "monthly_active_users.html", domain: "text", content_text: page }), path: "monthly_active_users.html", shows: /Monthly active users/ },
  "Chariox write_artifact (text), the content cut short by the kernel": {
    update: completed("chariox.write_artifact", { path: "monthly_active_users.html", domain: "text", content_text: `${page}\n\n[chariox: output truncated, omitted 5200 bytes]\n` }),
    path: "monthly_active_users.html", shows: /Monthly active users/,
  },
  "Chariox write_artifact (binary)": { update: completed("chariox.write_artifact", { path: "logo.png", domain: "opaque", content_base64: "iVBORw0KGgo=" }), path: "logo.png", shows: /binary/i },
  "Chariox write_artifact, namespaced by the MCP bridge": { update: completed("mcp__chariox__chariox_write_artifact", { path: "a.txt", content_text: "hello\n" }), path: "a.txt", shows: /hello/ },
  "Chariox edit_artifact": { update: completed("chariox.edit_artifact", { path: "a.txt", old_text: "old", new_text: "new" }), path: "a.txt", shows: /new/ },
  "Chariox patch_artifact": { update: completed("chariox.patch_artifact", { patch_text: "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** End Patch" }), path: "a.txt", shows: /new/ },
  "Chariox delete_artifact": { update: completed("chariox.delete_artifact", { path: "a.txt" }), path: "a.txt", shows: /./ },
  "Chariox move_artifact": { update: completed("chariox.move_artifact", { from_path: "a.txt", to_path: "b.txt" }), path: "a.txt", shows: /b\.txt/ },
}

function opened(display: ToolDisplay): string {
  return display.blocks.map((block) => {
    if (block.kind === "patch") return block.files.map((file) => `${file.filePath}\n${file.note ?? ""}\n${file.diff ?? ""}`).join("\n")
    if (block.kind === "text" || block.kind === "code") return block.text
    return JSON.stringify(block)
  }).join("\n")
}

for (const [name, { update, path, shows }] of Object.entries(shapes)) {
  test(`a file-write row opens to its path and its change: ${name}`, () => {
    const display = formatToolDisplay(update)
    const body = opened(display)
    assert.ok(display.blocks.length > 0, "the opened row has a body")
    assert.ok(body.trim().length > 0, "the body is not blank")
    assert.ok(`${display.title}\n${body}`.includes(path), `names ${path}: ${display.title}`)
    assert.match(body, shows)
  })
}

test("an oversized new file opens to a preview of its start and says how large it is", () => {
  const big = Array.from({ length: 4000 }, (_, i) => `<p>row ${i}</p>`).join("\n")
  const display = formatToolDisplay(completed("chariox.write_artifact", { path: "big.html", domain: "text", content_text: big }))
  const patch = display.blocks.find((block) => block.kind === "patch")
  assert.ok(patch && patch.kind === "patch")
  assert.equal(patch.files[0]!.filePath, "big.html")
  assert.match(patch.files[0]!.previewLines.map((line) => line.text).join("\n"), /row 0/)
})

test("a write whose content is unknown still opens to the file it names", () => {
  for (const update of [
    completed("chariox.write_artifact", { path: "monthly_active_users.html", domain: "text" }),
    completed("Write", { file_path: "/w/chart.html" }),
    completed("chariox.delete_artifact", { path: "old.txt" }),
  ]) {
    const display = formatToolDisplay(update)
    const patch = display.blocks.find((block) => block.kind === "patch")
    assert.ok(patch && patch.kind === "patch" && patch.files.length === 1, JSON.stringify(update))
    assert.ok(patch.files[0]!.previewLines.length > 0)
  }
})

// Review of c#616: the Chariox MCP bridge names a tool "chariox.write_artifact", "chariox_write_artifact" or, through a provider's MCP client,
// "mcp__chariox__chariox_write_artifact". Every spelling reads the change the call's own output reports, not the input.
for (const verb of ["write", "edit", "delete", "move"]) {
  for (const name of [`chariox.${verb}_artifact`, `chariox_${verb}_artifact`, `mcp__chariox__chariox_${verb}_artifact`, `mcp__chariox__${verb}_artifact`]) {
    test(`${name} reads the change in its output`, () => {
      const output = JSON.stringify({ applied: true, change: { path: "src/app.ts", kind: "update", diff: "--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-before\n+FROM_OUTPUT" } })
      const files = readApplyPatchFiles({ id: "c", tool: name, status: "completed", input: { path: "src/app.ts", content_text: "FROM_INPUT", from_path: "src/app.ts", to_path: "src/b.ts" }, output })
      assert.equal(files.length, 1)
      assert.match(files[0]!.diff ?? "", /FROM_OUTPUT/)
    })
  }
}

// The same calls, titled and iconed: the live row of a binary write through Claude's MCP client read "Edited out/logo.png".
for (const [name, title, icon] of [
  ["chariox.write_artifact", "Wrote out/logo.png", "edit"],
  ["mcp__chariox__write_artifact", "Wrote out/logo.png", "edit"],
  ["mcp__chariox__chariox_write_artifact", "Wrote out/logo.png", "edit"],
  ["mcp__chariox__chariox_delete_artifact", "Deleted out/logo.png", "edit"],
  ["mcp__chariox__chariox_move_artifact", "Moved out/logo.png", "edit"],
] as const) {
  test(`${name} is titled by what it did`, () => {
    const display = formatToolDisplay({ id: "c", tool: name, status: "completed", input: { path: "out/logo.png", content_base64: "iVBORw0KGgo=", from_path: "out/logo.png", to_path: "out/b.png" } })
    assert.match(display.title, new RegExp(`^${title.replace(/[./]/g, "\\$&")}`))
    assert.equal(display.icon, icon)
  })
}
