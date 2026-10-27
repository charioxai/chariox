import assert from "node:assert/strict"
import test from "node:test"

import { diffStat, diffView, parseUnifiedDiff } from "./diff-view.js"

// MP-08 / MP-10 (PANE-09, R16): Codex-style diff view model.
test("parses a git diff into files, numbered lines, counts and hunk gaps", () => {
  const files = parseUnifiedDiff([
    "diff --git a/README.md b/README.md",
    "index 1111111..2222222 100644",
    "--- a/README.md",
    "+++ b/README.md",
    "@@ -1,3 +1,3 @@",
    " # title",
    "-before: original",
    "+before: updated",
    " keep",
    "@@ -10,2 +10,3 @@ section",
    " ten",
    "+eleven",
    "--- a removed line starting with dashes",
    "\\ No newline at end of file",
  ].join("\n"))

  assert.equal(files.length, 1)
  const [file] = files
  assert.equal(file!.path, "README.md")
  assert.equal(file!.change, "update")
  assert.equal(file!.additions, 2)
  assert.equal(file!.deletions, 2)
  assert.deepEqual(file!.rows, [
    { kind: "line", line: "context", number: 1, text: "# title" },
    { kind: "line", line: "removed", number: 2, text: "before: original" },
    { kind: "line", line: "added", number: 2, text: "before: updated" },
    { kind: "line", line: "context", number: 3, text: "keep" },
    { kind: "gap" },
    { kind: "line", line: "context", number: 10, text: "ten" },
    { kind: "line", line: "added", number: 11, text: "eleven" },
    { kind: "line", line: "removed", number: 11, text: "-- a removed line starting with dashes" },
    { kind: "meta", text: "No newline at end of file" },
  ])
  assert.deepEqual(diffView(files), { verb: "Edited", target: "README.md", additions: 2, deletions: 2, files })
})

test("multi-file, added, deleted and renamed files get Codex verbs", () => {
  const files = parseUnifiedDiff([
    "--- /dev/null", "+++ b/new.txt", "@@ -0,0 +1 @@", "+hello",
    "--- a/old.txt", "+++ /dev/null", "@@ -1 +0,0 @@", "-bye",
    "diff --git a/a.ts b/b.ts", "similarity index 90%", "rename from a.ts", "rename to b.ts",
  ].join("\n"))

  assert.deepEqual(files.map((file) => [file.change, file.path, file.previousPath, file.additions, file.deletions]), [
    ["add", "new.txt", null, 1, 0],
    ["delete", "old.txt", "old.txt", 0, 1],
    ["move", "b.ts", "a.ts", 0, 0],
  ])
  const view = diffView(files)
  assert.equal(`${view.verb} ${view.target} ${diffStat(view.additions, view.deletions)}`, "Edited 3 files (+1 −1)")
  assert.equal(diffView([files[0]!]).verb, "Added")
  assert.equal(diffView([files[2]!]).target, "a.ts → b.ts")
})

test("a bare +/- fence has no path or numbers and is labelled Diff", () => {
  const files = parseUnifiedDiff("-old line\n+new line\n context")
  assert.deepEqual(files[0]!.rows.map((row) => row.kind === "line" ? [row.line, row.number] : row.kind), [["removed", null], ["added", null], ["context", null]])
  const view = diffView(files)
  assert.equal(`${view.verb} ${diffStat(view.additions, view.deletions)}`, "Diff (+1 −1)")
})
