// MP-08 / MP-10 / MP-11: skill items cross the same home prompt admission.
import assert from "node:assert/strict"
import test from "node:test"
import { LocalIpcError, type LocalIpcClient } from "../ipc.js"
import type { RuntimeProviderRun } from "../cli-types.js"
import { handleCodexNativeTurnStart } from "./codex-turn-submission.js"

async function submit(off = false) {
  const requests: Record<string, any>[] = [], replies: any[] = []
  const client = { send: async (request: Record<string, any>) => {
    requests.push(request)
    if (request.RelayStatus === null) return { RelayStatus: { status: { capabilities: ["skills_v2_runtime"] } } }
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

// MP-08 / MP-10 / MP-11: replay the foundation dispatcher's refusal, rather
// than pretending the protocol foundation implements catalog/submit.
const unavailable = () => new LocalIpcError("handle kernel response",
  "local transport `skills.v2` failed: Skills v2 catalog service is not available in this protocol foundation build",
  "local_transport_error", true)
for (const queued of [false, true]) test(`ordinary native text retains home admission without v2 (${queued ? "queued" : "started"})`, async () => {
  const requests: any[] = [], replies: any[] = [], started: string[] = []
  const client = { send: async (request: any) => {
    requests.push(request)
    if (request.RelayStatus === null) return { RelayStatus: { status: { capabilities: ["skills_v2_runtime"] } } }
    if (request.GetSessionState) return { SessionState: { session: { project_id: null } } }
    if (request.SkillsV2) throw unavailable()
    if (request.SubmitPrompt) return { PromptSubmitted: { outcome: {
      [queued ? "Queued" : "Started"]: { prompt: { id: "ordinary-prompt" } },
    } } }
    throw new Error("unexpected request")
  } } as unknown as LocalIpcClient
  const run = { id: "run" } as RuntimeProviderRun
  await handleCodexNativeTurnStart({ id: 18, method: "turn/start", params: { threadId: "display", input: [
    { type: "text", text: "explain this code" }, { type: "image", url: "https://example.org/image.png" },
  ] } }, { client, sessionId: "session", attachmentId: "attachment", agentId: "agent",
    bindState: { promise: Promise.resolve(run), run }, inlineLocalAttachments: false,
    onPromptStarted: id => started.push(id), debug: () => {} }, reply => replies.push(reply))
  assert.equal(replies[0].error, undefined)
  assert.equal(replies[0].result.turn.id, "chariox-native-ordinary-prompt")
  assert.deepEqual(requests.find(r => r.SubmitPrompt)?.SubmitPrompt, {
    session_id: "session", attachment_id: "attachment", target_agent_id: "agent", prompt: "explain this code\n",
    attachments: [{ url: "https://example.org/image.png", mime: "image/png", filename: "image.png" }],
  })
  assert.equal(requests.some(r => r.SkillsV2?.action === "submit"), false, "never retry a submission")
  assert.deepEqual(started, queued ? [] : ["ordinary-prompt"])
})
for (const failure of [unavailable(), new Error("catalog authorization denied"), new Error("socket closed")]) {
  test(`selected native skill never falls back on ${("code" in failure ? failure.code : failure.message)}`, async () => {
    const requests: any[] = [], replies: any[] = []
    const client = { send: async (request: any) => {
      requests.push(request)
    if (request.RelayStatus === null) return { RelayStatus: { status: { capabilities: ["skills_v2_runtime"] } } }
      if (request.GetSessionState) return { SessionState: { session: { project_id: null } } }
      throw failure
    } } as unknown as LocalIpcClient
    const run = { id: "run" } as RuntimeProviderRun
    await handleCodexNativeTurnStart({ id: 19, method: "turn/start", params: { threadId: "display", input: [
      { type: "skill", name: "only-in-chariox", path: "/untrusted/SKILL.md" },
    ] } }, { client, sessionId: "session", attachmentId: "attachment", agentId: "agent",
      bindState: { promise: Promise.resolve(run), run }, inlineLocalAttachments: false, debug: () => {} }, reply => replies.push(reply))
    assert.ok(replies[0].error)
    assert.equal(requests.some(r => r.SubmitPrompt || r.SkillsV2?.action === "submit"), false)
  })
}

for (const failure of [new Error("catalog authorization denied"), new Error("socket closed"),
  new LocalIpcError("handle kernel response", "local transport `skills.v2` failed: Skills v2 catalog service is not available in this protocol foundation build", "authorization_denied", false)]) {
  test(`ordinary prompt preserves admission errors: ${failure.message}`, async () => {
    const requests: any[] = [], replies: any[] = []
    const client = { send: async (request: any) => {
      requests.push(request)
    if (request.RelayStatus === null) return { RelayStatus: { status: { capabilities: ["skills_v2_runtime"] } } }
      if (request.GetSessionState) return { SessionState: { session: { project_id: null } } }
      throw failure
    } } as unknown as LocalIpcClient
    const run = { id: "run" } as RuntimeProviderRun
    await handleCodexNativeTurnStart({ id: 20, method: "turn/start", params: { threadId: "display", input: [
      { type: "text", text: "ordinary text" },
    ] } }, { client, sessionId: "session", attachmentId: "attachment", agentId: "agent",
      bindState: { promise: Promise.resolve(run), run }, inlineLocalAttachments: false, debug: () => {} }, reply => replies.push(reply))
    assert.ok(replies[0].error)
    assert.equal(requests.filter(r => r.SubmitPrompt).length, 1)
    assert.equal(requests.some(r => r.SkillsV2), false)
  })
}
test("a v2 submission refusal never retries through legacy admission", async () => {
  const requests: any[] = [], replies: any[] = []
  const client = { send: async (request: any) => {
    requests.push(request)
    if (request.RelayStatus === null) return { RelayStatus: { status: { capabilities: ["skills_v2_runtime"] } } }
    if (request.GetSessionState) return { SessionState: { session: { project_id: null } } }
    if (request.SkillsV2?.action === "catalog") return { SkillsV2: { result: {
      kind: "catalog", revision: 1, skills: [], packages: [], native: [], metadata_budget_percent: 1,
    } } }
    throw unavailable()
  } } as unknown as LocalIpcClient
  const run = { id: "run" } as RuntimeProviderRun
  await handleCodexNativeTurnStart({ id: 21, method: "turn/start", params: { threadId: "display", input: [
    { type: "text", text: "use $review" },
  ] } }, { client, sessionId: "session", attachmentId: "attachment", agentId: "agent",
    bindState: { promise: Promise.resolve(run), run }, inlineLocalAttachments: false, debug: () => {} }, reply => replies.push(reply))
  assert.ok(replies[0].error)
  assert.equal(requests.filter(r => r.SkillsV2?.action === "submit").length, 1)
  assert.equal(requests.some(r => r.SubmitPrompt), false)
})

test("advertised v2 typed selection cannot fall back when catalog fails", async () => {
  const requests: any[] = [], replies: any[] = []
  const client = { send: async (request: any) => {
    requests.push(request)
    if (request.RelayStatus === null) return { RelayStatus: { status: { capabilities: ["skills_v2_runtime"] } } }
    if (request.GetSessionState) return { SessionState: { session: { project_id: null } } }
    if (request.SkillsV2) throw unavailable()
    return { PromptSubmitted: { outcome: { Started: { prompt: { id: "must-not-run" } } } } }
  } } as unknown as LocalIpcClient
  const run = { id: "run" } as RuntimeProviderRun
  await handleCodexNativeTurnStart({ id: 22, method: "turn/start", params: { threadId: "display", input: [
    { type: "text", text: "use $only-in-chariox" },
  ] } }, { client, sessionId: "session", attachmentId: "attachment", agentId: "agent",
    bindState: { promise: Promise.resolve(run), run }, inlineLocalAttachments: false, debug: () => {} }, reply => replies.push(reply))
  assert.ok(replies[0].error)
  assert.equal(requests.some(r => r.SubmitPrompt || r.SkillsV2?.action === "submit"), false)
})

// MP-08 / MP-10 / MP-11: protocol 533 foundation has no runtime capability.
for (const chip of [false, true]) test(`foundation keeps native skill admission (${chip ? "chip" : "typed"})`, async () => {
  const requests: any[] = [], replies: any[] = []
  const client = { send: async (request: any) => {
    requests.push(request)
    if (request.RelayStatus === null) return { RelayStatus: { status: { capabilities: ["skills_v2_contract"] } } }
    if (request.SkillsV2) throw unavailable()
    if (request.SubmitPrompt) return { PromptSubmitted: { outcome: { Started: { prompt: { id: "native-prompt" } } } } }
    throw new Error("unexpected request")
  } } as unknown as LocalIpcClient
  const run = { id: "run" } as RuntimeProviderRun
  await handleCodexNativeTurnStart({ id: 23, method: "turn/start", params: { threadId: "display", input: chip
    ? [{ type: "skill", name: "review", path: "/untrusted/SKILL.md" }]
    : [{ type: "text", text: "$review inspect source" }],
  } }, { client, sessionId: "session", attachmentId: "attachment", agentId: "agent",
    bindState: { promise: Promise.resolve(run), run }, inlineLocalAttachments: false, debug: () => {} }, reply => replies.push(reply))
  assert.equal(replies[0].error, undefined)
  assert.equal(replies[0].result.turn.id, "chariox-native-native-prompt")
  assert.equal(requests.some(r => r.SkillsV2), false)
  assert.equal(requests.filter(r => r.SubmitPrompt).length, 1)
  assert.equal(requests.find(r => r.SubmitPrompt).SubmitPrompt.prompt, chip ? "$review\n" : "$review inspect source\n")
  assert.equal(JSON.stringify(requests).includes("/untrusted"), false)
})

// MP-08 / MP-10 / MP-11: status errors are not a false capability result.
test("native skill status failure cannot bypass admission through the old path", async () => {
  const requests: any[] = [], replies: any[] = []
  const client = { send: async (request: any) => { requests.push(request); throw new Error("status authorization denied") } } as unknown as LocalIpcClient
  const run = { id: "run" } as RuntimeProviderRun
  await handleCodexNativeTurnStart({ id: 24, method: "turn/start", params: { threadId: "display", input: [
    { type: "text", text: "$review source" },
  ] } }, { client, sessionId: "session", attachmentId: "attachment", agentId: "agent",
    bindState: { promise: Promise.resolve(run), run }, inlineLocalAttachments: false, debug: () => {} }, reply => replies.push(reply))
  assert.match(replies[0].error.message, /status authorization denied/)
  assert.deepEqual(requests, [{ RelayStatus: null }])
})
