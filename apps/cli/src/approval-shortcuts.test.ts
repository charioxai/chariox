import assert from "node:assert/strict"
import test from "node:test"
import { approvalShortcutLabel, isApprovalShortcut } from "./approval-shortcuts.js"

test("approval hints name the actual Mac function key and terminal alternative", () => {
  assert.equal(approvalShortcutLabel("darwin"), "F8 (fn+F8 on Mac) or Ctrl+G")
  assert.equal(approvalShortcutLabel("linux"), "F8 or Ctrl+G")
  assert.equal(isApprovalShortcut({ name: "g", ctrl: true }), true)
  assert.equal(isApprovalShortcut({ name: "T", ctrl: true }), false)
})
