// MP-08 / MP-10 / MP-11: shared TUI composer uses the same context fence.
import assert from "node:assert/strict";
import { test } from "node:test";
import { SkillsComposerController } from "./skills-composer-controller.js";
const selection = {source:"native" as const,entry_id:"n",name:"review",provider:"codex",account_profile:"a",execution_id:"local",inventory_revision:"v1"};
// MP-08 / MP-10 / MP-11: exact reviewer A -> B -> A context transitions.
for (const pendingB of [false, true]) {
    test(`returning to A reloads its catalog and fences B (${pendingB ? "pending B" : "B picker never opened"})`, async () => {
        let agent = "A", text = "$", cursor = 1;
        const reads: string[] = [];
        let releaseB: ((value: unknown) => void) | undefined;
        const catalog = (name: string) => ({ SkillsV2: { result: { kind: "catalog", revision: 1, skills: [], packages: [], native: [{ selection: { ...selection, name, entry_id: name }, description: name, exposure: "confirmed" }], metadata_budget_percent: 1 } } });
        const controller = new SkillsComposerController({
            context: () => ({ key: agent, agentId: agent, sessionId: null, projectId: null }),
            provider: () => "codex", read: () => ({ text, cursor }), write: () => {}, changed: () => {}, render: () => {},
            send: async request => { if ("RelayStatus" in (request as object)) return {RelayStatus:{status:{capabilities:["skills_v2_runtime"]}}}; const requested = agent; reads.push(requested); return requested === "B" ? new Promise(resolve => { releaseB = resolve; }) : catalog(requested); },
        });
        controller.sync(text);
        await new Promise(r => setImmediate(r));
        assert.match(controller.items()!.rows[0]!.label, /\$A/);
        agent = "B";
        text = pendingB ? "$" : "ordinary text"; cursor = text.length;
        controller.sync(text);
        if (pendingB) await new Promise(r => setImmediate(r));
        agent = "A"; text = "$"; cursor = 1;
        controller.sync(text);
        await new Promise(r => setImmediate(r));
        assert.deepEqual(reads, pendingB ? ["A", "B", "A"] : ["A", "A"]);
        releaseB?.(catalog("B"));
        await new Promise(r => setImmediate(r));
        assert.match(controller.items()!.rows[0]!.label, /\$A/);
        assert.equal(controller.client.key, "A");
    });
}

// MP-08 / MP-10 / MP-11: no v2 picker requests without kernel runtime admission.
test("foundation capability absence leaves dollar input on the ordinary composer", async () => {
    const requests: unknown[] = [];
    const text = "$review";
    const controller = new SkillsComposerController({context: () => ({key:"foundation",agentId:"agent",sessionId:null,projectId:null}),provider:()=>"codex",read:()=>({text,cursor:text.length}),write:()=>{},changed:()=>{},render:()=>{},send:async request=>{requests.push(request);return {RelayStatus:{status:{capabilities:[]}}}}});
    controller.sync(text);
    await new Promise(r=>setImmediate(r));
    assert.deepEqual(requests,[{RelayStatus:null}]);
    assert.equal(controller.items(),null);
    assert.equal(controller.client.catalog,null);
});
