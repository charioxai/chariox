// MP-08 / MP-10 / MP-11: native skill chips must reach home resolution.
import assert from "node:assert/strict"
import test from "node:test"
import { extractCodexPrompt, extractCodexSkillNames } from "./codex-prompt.js"

test("skill-only input preserves its chip in home history without trusting paths", () => {
  const params = { input: [{ type: "skill", name: "review", path: "/untrusted/SKILL.md" }] }
  assert.deepEqual(extractCodexSkillNames(params), ["review"])
  assert.equal(extractCodexPrompt(params), "$review\n")
})
test("skill chip already present in text is delivered once", () => {
  const params = { input: [{ type: "text", text: "$review inspect" }, { type: "skill", name: "review", path: "/ignored" }] }
  assert.equal(extractCodexPrompt(params), "$review inspect\n")
  assert.deepEqual(extractCodexSkillNames(params), ["review"])
})
test("invalid skill input fails rather than silently dropping intent", () => {
  assert.throws(() => extractCodexSkillNames({ input: [{ type: "skill", name: "../bad" }] }), /invalid Codex skill/)
})

test("explicit native chips retain the kernel catalog's valid mixed-case and long names", () => {
  const name = "Review" + "a".repeat(80)
  assert.deepEqual(extractCodexSkillNames({ input: [{ type: "skill", name, path: "/discard" }] }), [name])
})
