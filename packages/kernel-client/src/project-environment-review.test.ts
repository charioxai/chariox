// MP-08 / MP-10 / MP-11: Both clients project kernel decisions; values stay in volatile replies.
import assert from "node:assert/strict"
import test from "node:test"
import { projectEnvironmentReviewLines, type ProjectEnvironmentReview } from "./project-environment-review.js"

const review: ProjectEnvironmentReview = {
  schema_version: 1, project_name: "Web fixture", target_name: "Fresh slice", expanded: false,
  unattended: false, changed_only: false,
  rows: [{ label: "Code", summary: "main@abc123 · 1 uncommitted" }, {label: "Environment", summary: "4 variables · 4 secrets hidden"}],
  files: [{ id: "file-0", path: "CLAUDE.local.md", bring: true, reason: "personal project instructions", changed: true }],
  inputs: [{id: "missing-1", name: "API_TOKEN", workspace_id: "web", missing: true, secret: true,
    source: "missing", changed: true, uses: [{path: "src/app.ts", line: 4}]}],
}

test("MP-08 collapsed review shows summaries and Needs you without hidden details", () => {
  const lines = projectEnvironmentReviewLines(review)
  assert.ok(lines.some(line => line.includes("Environment: 4 variables")))
  assert.ok(lines.some(line => line.includes("API_TOKEN")))
  assert.ok(!lines.some(line => line.includes("CLAUDE.local.md")))
})
test("MP-08 details preserve server decisions and one-line reasons", () => {
  const lines = projectEnvironmentReviewLines({...review, expanded: true})
  assert.ok(lines.some(line => line.includes("Bring CLAUDE.local.md — personal project instructions")))
  assert.ok(lines.some(line => line.includes("src/app.ts:4")))
})
test("MP-08 changed-only review hides old decisions", () => {
  const lines = projectEnvironmentReviewLines({...review, expanded: true, changed_only: true, files: review.files.map(file => ({...file, changed: false}))})
  assert.ok(!lines.some(line => line.includes("CLAUDE.local.md")))
})
test("MP-08 review projection contains names and use sites only", () => {
  assert.ok(!Object.prototype.hasOwnProperty.call(review.inputs[0], "value"))
  assert.ok(projectEnvironmentReviewLines({...review, expanded: true}).some(line => line.includes("secret hidden")))
})
test("MP-08 / MP-10 / MP-11 unattended projection shows the saved setup summary", () => {
  assert.ok(projectEnvironmentReviewLines({...review, unattended: true})[0] === "Saved setup applied")
})
