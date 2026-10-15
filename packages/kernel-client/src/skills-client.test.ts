// MP-08 / MP-10 / MP-11 — regression checks, not live acceptance.
import assert from "node:assert/strict";
import { test } from "node:test";
import { skillPickerRows, skillGroups, skillTokenAtCursor, insertSkillChip, reconcileSkillChips, SkillsClient } from "./skills-client.js";
import type { SkillProjection, NativeSkillEntry, SkillSelection } from "./kernel-types-skills-v2.js";
const skill = (state = "auto", delivery: unknown = { kind: "chariox" }): SkillProjection => ({ skill: { id: "s1", name: "review", scope: { kind: "chariox" }, package_id: null, description: "Review", digest: "abc", default_state: "auto", state_override: null, files: [], provenance: {}, compatibility_notes: [] }, effective_state: state, project_override: null, delivery, also_in_provider: delivery && (delivery as {
        kind: string;
    }).kind === "provided_by" ? "codex" : null } as unknown as SkillProjection);
const native = (provider = "codex", entry_id = "n1"): NativeSkillEntry => ({ selection: { source: "native", name: "review", entry_id, provider, account_profile: "acct", execution_id: "local", inventory_revision: "r1" }, description: "Native review", exposure: "confirmed", default_state: "auto", digest: "abc", complete_manifest: true, supports_load_ack: true, discovery_error: null });
test("picker dedupes confirmed equal copies, preserves conflicting explicit sources and hides Off", () => {
    assert.equal(skillPickerRows([skill("off")], [], 3, null).length, 0);
    const rows = skillPickerRows([skill("manual", { kind: "provided_by", provider: "codex", entry_id: "n1" })], [native(), native("claude")], 3, { provider: "codex", account: "acct", execution: "local" });
    assert.equal(rows.length, 1);
    assert.match(rows[0]!.label, /also in Codex/);
    assert.equal(skillPickerRows([skill()], [native()], 3, { provider: "codex", account: "acct", execution: "local" }).length, 2);
    assert.equal(skillPickerRows([skill()], [native()], 3, null).length, 1);
});
test("packages include empty custom groups, Ungrouped and Mixed states", () => {
    const member = skill("manual");
    member.skill = { ...member.skill, id: "s2", package_id: "p" };
    const other = skill();
    other.skill = { ...other.skill, package_id: "p" };
    const groups = skillGroups([member, other, skill()], [{ id: "p", name: "Reviews", custom: true }, { id: "empty", name: "Empty", custom: true }] as never);
    assert.equal(groups.find(g => g.id === "p")!.state, "mixed");
    assert.equal(groups.find(g => g.id === "empty")!.members.length, 0);
    assert.equal(groups.find(g => g.id === null)!.members.length, 1);
});
test("cursor token respects code, escaping, shell variables and suffix text", () => {
    assert.deepEqual(skillTokenAtCursor("Read $rev after", 9), { start: 5, end: 9, query: "rev" });
    assert.deepEqual(skillTokenAtCursor("$review after",3),{start:0,end:7,query:"re"});
    for (const text of ["\\$rev", "`$rev", "```\n$rev", "~~~\n$rev", "``$rev", "$HOME", "cost $20"])
        assert.equal(skillTokenAtCursor(text, text.length), null);
});
test("typed chip preserves text and source; focus changes invalidate native chips only", () => {
    const s: SkillSelection = { source: "chariox", skill_id: "s1", name: "review", catalog_revision: 3 };
    const result = insertSkillChip("Read $rev after", { start: 5, end: 9, query: "rev" }, s);
    assert.equal(result.text, "Read $review after");
    assert.equal(result.chip.selection, s);
    assert.equal(reconcileSkillChips([result.chip], result.text, [], null)[0]!.invalid, null);
    const n = insertSkillChip("$", { start: 0, end: 1, query: "" }, native().selection);
    assert.match(reconcileSkillChips([n.chip], n.text, [native("claude")], null)[0]!.invalid!, /Reselect/);
    assert.equal(reconcileSkillChips([result.chip], "Read  after", [], null).length, 0);
});
test("client rejects wrong variants, fences late catalog replies and previews before mutation", async () => {
    let release!: (r: unknown) => void;
    const requests: unknown[] = [];
    const client = new SkillsClient(async (request) => { requests.push(request); return new Promise(resolve => { release = resolve; }); });
    const pending = client.refresh({ key: "old", projectId: null, agentId: null, sessionId: null });
    client.invalidate("new");
    release({ SkillsV2: { result: { kind: "catalog", revision: 1, skills: [], packages: [], native: [], metadata_budget_percent: 1 } } });
    await pending;
    assert.equal(client.catalog, null);
    const bad = new SkillsClient(async () => ({ SkillsV2: { result: { kind: "published" } } }));
    await assert.rejects(() => bad.refresh({ key: "x", projectId: null, agentId: null, sessionId: null }), /catalog/);
    const order: string[] = [];
    const good = new SkillsClient(async (r) => { order.push(r.SkillsV2.action); return { SkillsV2: { result: r.SkillsV2.action === "preview" ? { kind: "preview", expected_revision: 1, affected: [], collisions: ["name exists"] } : { kind: "published", revision: 2 } } }; });
    await assert.rejects(() => good.preview({ scope: { kind: "chariox" }, expected_revision: 1, actions: [] }), /name exists/);
    assert.deepEqual(order, ["preview"]);
});

// MP-08 / MP-10 / MP-11: a review receipt is required even for non-source batches.
test("missing and expired home reviews cannot publish", async () => {
  const mutation={scope:{kind:"chariox" as const},expected_revision:1,actions:[]}
  for(const receipt of [undefined,{id:"expired",expires_at_ms:Date.now()-1,mutation_sha256:"hash",sources:[]}]) {
    const calls:unknown[]=[]
    const client=new SkillsClient(async r=>{calls.push(r);return {SkillsV2:{result:{kind:"published",revision:2}}}})
    await assert.rejects(()=>(client as any).publish(mutation,receipt),/review.*again|receipt/i)
    assert.equal(calls.length,0)
  }
})

// MP-08 / MP-10 / MP-11: equal content folds even without provider delivery.
test("Manual Chariox plus identical Auto provider folds once, but Off and conflicts retain native", () => {
    const focus = { provider: "codex", account: "acct", execution: "local" };
    const installed = skill("manual");
    installed.also_in_provider = "codex";
    installed.skill.digest = "abc";
    const rows = skillPickerRows([installed], [native()], 3, focus);
    assert.equal(rows.length, 1);
    assert.match(rows[0]!.label, /also in Codex/);
    const conflict = { ...native(), digest: "different" };
    assert.equal(skillPickerRows([installed], [conflict], 3, focus).length, 2);
    installed.effective_state = "off";
    const off = skillPickerRows([installed], [native()], 3, focus);
    assert.equal(off.length, 1);
    assert.match(off[0]!.label, /From Codex/);
});
