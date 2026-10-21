// MP-08 / MP-10 / MP-11: source-aware native proxy regression checks.
import assert from "node:assert/strict"
import test from "node:test"
import { codexSkillSelections, projectCodexSkillList } from "./skills-v2.js"
import type { SkillProjection, SkillsV2Response, NativeSkillEntry } from "@chariox/kernel-client/kernel-types-skills-v2"
type Catalog = Extract<SkillsV2Response, { kind: "catalog" }>
const row = (state: "auto" | "manual" | "off" = "auto"): SkillProjection => ({
  skill: { id: "skill-review", name: "review", scope: { kind: "chariox" }, package_id: null,
    description: "Review current source", short_description: null, default_state: "auto", state_override: null,
    digest: "a".repeat(64), declared_version: null, files: [], compatibility_notes: [],
    provenance: { kind: "content", source_machine: null, locator: null, source_revision: null,
      provider: null, account_profile: null, installed_at_ms: 1, updated_at_ms: 1 } },
  effective_state: state, project_override: null, delivery: { kind: "chariox" }, also_in_provider: null,
})
const native = (): NativeSkillEntry => ({
  selection: { source: "native", provider: "codex", name: "review", entry_id: "native-review", account_profile: "selected",
    execution_id: "worker", inventory_revision: "current" },
  description: "Native review", digest: "a".repeat(64), exposure: "confirmed", default_state: "auto",
  complete_manifest: true, supports_load_ack: true, discovery_error: null,
})
const catalog = (skills = [row()], entries = [native()]): Catalog => ({
  kind: "catalog", revision: 7, skills, packages: [], native: entries, metadata_budget_percent: 1,
})
test("Chariox identity wins conflicting native input; paths never enter the selection", () => {
  assert.deepEqual(codexSkillSelections(["review"], catalog()), [
    { source: "chariox", skill_id: "skill-review", name: "review", catalog_revision: 7 },
  ])
})
test("Off cannot fall through to native copy and unverified native entries cannot load", () => {
  assert.throws(() => codexSkillSelections(["review"], catalog([row("off")])), /Off/)
  const entry = native(); entry.exposure = "unverified"
  assert.throws(() => codexSkillSelections(["review"], catalog([], [entry])), /unavailable/)
})
test("an offered native completion stays selectable when its Chariox namesake is Off", () => {
  const entry = native()
  assert.deepEqual(codexSkillSelections(["review"], catalog([row("off")], [entry]), new Set(["review"])), [entry.selection])
  assert.throws(() => codexSkillSelections(["review"], catalog([row("off")], [entry])), /Off/)
  assert.throws(() => codexSkillSelections(["review"], catalog([row("off")], []), new Set(["review"])), /Off/)
})
test("native-only inputs carry the kernel-issued account/execution/inventory reference", () => {
  const entry = native()
  assert.deepEqual(codexSkillSelections(["review"], catalog([], [entry])), [entry.selection])
})
test("projection exposes a Chariox-only name and replaces a conflict without a duplicate", () => {
  const upstream = { data: [{ cwd: "/actual", skills: [{ name: "review", description: "different bytes", path: "/native/review/SKILL.md" }], errors: [] }] }
  const projected = projectCodexSkillList(upstream, catalog()) as typeof upstream
  assert.equal(projected.data[0]!.skills.length, 1)
  assert.equal(projected.data[0]!.skills[0]!.path, "chariox-skill://skill-review/SKILL.md")
  assert.equal(upstream.data[0]!.skills[0]!.path, "/native/review/SKILL.md", "never mutate actual harness inventory")
  const absent = projectCodexSkillList({ data: [{ cwd: "/actual", skills: [], errors: [] }] }, catalog()) as typeof upstream
  assert.equal(absent.data[0]!.skills[0]!.name, "review")
})
test("exact native duplicate leaves one original native completion row", () => {
  const skill = row(); skill.delivery = { kind: "provided_by", provider: "codex", entry_id: "native-review" }
  const upstream = { data: [{ cwd: "/actual", skills: [{ name: "review", path: "/native/review/SKILL.md" }], errors: [] }] }
  assert.deepEqual(projectCodexSkillList(upstream, catalog([skill])), upstream)
})
