// MP-08 / MP-10 / MP-11: skill items cross the same home prompt admission.
import assert from "node:assert/strict"
import test from "node:test"
import type { LocalIpcClient } from "../ipc.js"
import type { RuntimeProviderRun } from "../cli-types.js"
import { handleCodexNativeTurnStart } from "./codex-turn-submission.js"

async function submit(off = false) {
  const requests: Record<string, any>[] = [], replies: any[] = []
  const client = { send: async (request: Record<string, any>) => {
    requests.push(request)
    if (request.GetSessionState) return { SessionState: { session: { project_id: "project" } } }
    if (request.SkillsV2?.action === "catalog") return { SkillsV2: { result: {
      kind: "catalog", revision: 9, skills: [{ skill: { id: "home-skill", name: "only-in-chariox" }, effective_state: off ? "off" : "manual" }], native: [],
    } } }
    if (request.SkillsV2?.action === "submit") return { SkillsV2: { result: {
      kind: "submitted", outcome: { Started: { prompt: { id: "admitted-prompt" } } },
    } } }
    throw new Error("unexpected request")
  } } as unknown as LocalIpcClient
  const run = { id: "run" } as RuntimeProviderRun
  await handleCodexNativeTurnStart({ id: 17, method: "turn/start", params: { threadId: "display", input: [
    { type: "text", text: "inspect source" }, { type: "skill", name: "only-in-chariox", path: "/attacker/credentials/SKILL.md" },
  ] } }, { client, sessionId: "session", attachmentId: "attachment", agentId: "agent",
    bindState: { promise: Promise.resolve(run), run }, inlineLocalAttachments: false, debug: () => {} }, reply => replies.push(reply))
  return { requests, replies }
}
test("native skill-only chip becomes a kernel-validated reference with original history label", async () => {
  const { requests, replies } = await submit()
  const request = requests.find(r => r.SkillsV2?.action === "submit")!.SkillsV2
  assert.equal(request.prompt, "inspect source\n$only-in-chariox\n")
  assert.deepEqual(request.selections, [{ source: "chariox", skill_id: "home-skill", name: "only-in-chariox", catalog_revision: 9 }])
  assert.equal(JSON.stringify(requests).includes("/attacker"), false)
  assert.equal(replies[0].result.turn.id, "chariox-native-admitted-prompt")
  assert.equal(requests.some(r => r.SubmitPrompt), false)
})
test("Off skill rejects the native turn before prompt admission", async () => {
  const { requests, replies } = await submit(true)
  assert.match(replies[0].error.message, /Off/)
  assert.equal(requests.some(r => r.SkillsV2?.action === "submit"), false)
})
