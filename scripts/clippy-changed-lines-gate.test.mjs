import assert from "node:assert/strict"
import test from "node:test"

import { lineIsChanged, parseChangedRanges } from "./clippy-changed-lines-gate.mjs"

test("parses added and modified Rust line ranges", () => {
  const ranges = parseChangedRanges(`diff --git a/src/one.rs b/src/one.rs
--- a/src/one.rs
+++ b/src/one.rs
@@ -4,0 +5,2 @@
+first
+second
@@ -10 +12 @@
-old
+new
`)

  assert.deepEqual(ranges.get("src/one.rs"), [[5, 6], [12, 12]])
  assert.equal(lineIsChanged(ranges, "src/one.rs", 5), true)
  assert.equal(lineIsChanged(ranges, "src/one.rs", 6), true)
  assert.equal(lineIsChanged(ranges, "src/one.rs", 7), false)
})

test("ignores deleted-only hunks and unrelated files", () => {
  const ranges = parseChangedRanges(`diff --git a/src/one.rs b/src/one.rs
--- a/src/one.rs
+++ b/src/one.rs
@@ -4,2 +4,0 @@
-first
-second
`)

  assert.equal(ranges.has("src/one.rs"), false)
  assert.equal(lineIsChanged(ranges, "src/two.rs", 4), false)
})
