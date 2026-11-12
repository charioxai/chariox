// MP-08 / MP-09 / MP-10 / MP-11 orchdrill r2: the extra scenarios of orchestrate-one-room.mjs that validate Landing 5's
// sub-agents on one kernel (AO-SA-04/05, AO-PE-03/04, AO-PC-01, AO-OR-01). Briefs for the real orchestrator, and the
// driver-side steps that act as the person (answers, a second Room, raw calls without an agent token).
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { listDescendantsRequest } from "../../packages/kernel-client/dist/ipc-agent-requests.js"

const spawnLine = (name, model, extra = "") => `agent spawn ${name} --provider claude --model ${model} --effort low --permission yolo${extra}`

// AO-SA-05: one sub-agent per user-only decision kind it can raise with its own tools.
export function kindsBrief({ R, model, work, payPort }) {
  return `# Orchestrator brief (drill, kinds scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). This is a drill of the product's approval cards: every child below only ASKS the person for something and the person decides on the card; nothing is granted, bypassed or run without that answer, and none of it needs your own approval. Use only your room commands (chariox room run_command) and chariox events tools. No /sudo. Do not ask the owner anything. Never retry a refused call more than once: record the exact answer. Never use shell sleeps or poll.
Keep ${join(work, "orchestrator-state.json")} updated after every step.
1. Spawn k1, k2, k3, k4, k5, k6 and k7 (k6 gets no work and is not subscribed) with \`${spawnLine("<name>", model)}\`.
2. FIRST subscribe to k1..k5 and k7 (chariox events_subscribe with each agent id), THEN send each text below to its child with the room command \`prompt <alias> "<text>"\` (the child runs it, you do not):
   k1: "Call the chariox request_credential_secret tool with these arguments: credential {id 'drill-key', injection {kind 'header', name 'X-Drill-Key', value '{{secret}}'}, use_binding {target {kind 'site', origin 'https://example.invalid'}}}, prompt {message 'Drill: type any text', timeout_sec 900}. When it returns reply K1DONE."
   k2: "Call the chariox manage_credential_vault tool with action popup. When it returns reply K2DONE."
   k3: "Call the chariox request_popup tool with ONLY these arguments: wait true, timeout_sec 900, title 'k3 passkey', message 'Approve?', choices [{id:'ok',label:'OK',reply:'ok'}], action {request: {ListSessions: null}}. When it returns reply K3DONE."
   k4: "Run these in your shell, one by one, and report each exact output: (1) /root/.chariox/dev/subval1/bin/chariox-cli access status (2) /root/.chariox/dev/subval1/bin/chariox-cli access request. Do not retry. When done reply K4DONE."
   k5: "Call the chariox load_kernel_browser tool with no arguments. When it returns reply K5DONE."
   k7: "Call the chariox request_credential_secret tool with these arguments: credential {id 'drill-card', injection {kind 'header', name 'X-Card', value '{{secret}}'}, use_binding {value_kind 'payment_card', target {kind 'site', origin 'http://127.0.0.1:${payPort}'}}}, prompt {message 'Drill: type any text', timeout_sec 900}. Then call the chariox http_request_with_credential tool with credential_id drill-card and url http://127.0.0.1:${payPort}/ . Report each exact answer. When done reply K7DONE."
3. Yield on ALL your registrations (chariox events_subscriptions lists them) with a deadline 15 minutes ahead and END YOUR TURN. If events_yield is refused, do not retry: just END YOUR TURN, because every event wakes you anyway.
4. On each wake: read the events (chariox events_inbox). For a child_decision event do NOT answer or pass it: only append the interaction_id, the child_alias and the question to the state file. For a child_finished event record the child and the outcome. Ack the events and wait again until k1..k5 and k7 have each finished. Do NOT write the report before that.
5. Write ${join(work, "orchestrator-report.json")}: per child the finish outcome, and one entry per child_decision event you ever received (an empty list is a valid answer). Then delete k1..k7 with \`agent delete <name>\` and reply ORCHDONE.
`
}

// AO-OR-01: one room agent, two sub-agents, to the end of a room task.
export function orchBrief({ R, model, work }) {
  return `# Orchestrator brief (drill, orch scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Run this room task with two sub-agents using only your room commands (chariox room run_command), chariox events tools and the Write tool for the state file. You NEVER run a worker's command yourself and never use the Bash tool: the workers do the work. No /sudo. Do not ask the owner anything. Never retry a refused call more than once: record the exact answer. Never use shell sleeps or poll.
Keep ${join(work, "orchestrator-state.json")} updated after every step and every wake (time, what woke you, what you did).
1. Spawn w1 with \`agent spawn w1 --provider claude --model ${model} --effort low --permission required\` (it must ask before it acts) and w2 with \`${spawnLine("w2", model)}\`.
2. FIRST subscribe to w1 and w2 (chariox events_subscribe with each agent id), THEN send each text below to its worker with the room command \`prompt <alias> "<text>"\` (the worker runs it, you do not):
   w1: "If a message from your parent arrives while you work, do what it asks. Use your shell tool to run exactly: echo ALPHA > w1.txt . Then use your shell tool to run exactly: for i in $(seq 1 25); do sleep 1; done . Then call the chariox request_credential_secret tool with these arguments: credential {id 'drill-key', injection {kind 'header', name 'X-Drill-Key', value '{{secret}}'}, use_binding {target {kind 'site', origin 'https://example.invalid'}}}, prompt {message 'Drill: type any text', timeout_sec 900}. Then use your shell tool to run exactly: echo DONE > w1-done.txt . Reply W1DONE."
   w2: "Use your shell tool to run exactly: until [ -e ${work}/never-appears ]; do sleep 1; done . Then reply W2DONE."
3. Yield on ALL your registrations (chariox events_subscriptions lists them) with a deadline 15 minutes ahead and END YOUR TURN. If events_yield is refused, do not retry: just END YOUR TURN, every event wakes you anyway.
4. On each wake read the events (chariox events_inbox) and act:
   - child_decision event (w1 asking to run its shell tool): run the room command \`agent answer <interaction_id> <id of the choice that allows the action>\` and record the exact typed answer. Never answer a decision that is not in a child_decision event.
   - FIRST wake only (any kind of event): steer w1 now with chariox send_agent_message to w1, urgent true: "Also write the word DELTA into steer.txt in your working directory." and STOP w2 with the room command \`agent cancel w2\`; record the answers.
   - child_finished event: record the child, the outcome kind and whether deadline_reached.
   Ack every event and wait again until w1 AND w2 each reported a child_finished. Do NOT write the report before that.
5. Write ${join(work, "orchestrator-report.json")} with: each wake (what woke you), the answers of agent answer, agent cancel and the steer, and each child_finished outcome. Then delete w1 and w2 with \`agent delete <name>\` and reply ORCHDONE.
`
}

// The person's side of the kinds scenario: record who sees each card, then answer it.
export async function kindsDriver(ctx) {
  const { a, R, orch, s, seen, note, attachedNow } = ctx
  const alias = new Map((s.agents ?? []).map(x => [x.id, x.alias]))
  const tree = await a.rpc(listDescendantsRequest(R, orch)).then(r => r.AgentTree.tree, () => null)
  const byId = new Map((tree?.nodes ?? []).map(n => [n.agent_id, n]))
  for (const it of s.active_interactions ?? []) {
    const open = seen.interactions.get(it.id)
    if (!open) continue
    const node = byId.get(it.agent_id)
    const who = node?.alias ?? alias.get(it.agent_id) ?? it.agent_id
    if (seen.shown.get(it.id) === undefined) {
      seen.shown.set(it.id, it.shown_in_agent_id ?? null)
      note("kind-card", { id: it.id, asker: who, shown_in: it.shown_in_agent_id === orch ? "orchestrator" : it.shown_in_agent_id ?? null, asker_state: node?.state, asker_needs_user: node?.needs_user, title: it.title, secret: it.custom_choice?.input_kind === "secret", choices: (it.choices ?? []).map(c => c.id), copies: (s.active_interactions ?? []).filter(x => x.id === it.id).length })
    }
    if (Date.now() - open < 12000) continue
    const pick = (it.choices ?? []).find(c => /^(cancel|deny|decline|no|reject)/i.test(c.id)) ?? (it.choices ?? []).find(c => !c.requires_passkey) ?? it.choices?.[0]
    const reply = it.custom_choice?.input_kind === "secret" ? { choice_id: it.custom_choice.id, custom_reply: "drill-not-a-real-secret" } : { choice_id: pick?.id }
    await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, ...reply } }).then(() => { seen.interactions.set(it.id, 0); note("kind-answered", { id: it.id, asker: who, choice: reply.choice_id }) }, e => { seen.interactions.set(it.id, Date.now()); note("kind-answer-refused", { id: it.id, asker: who, message: e.message.slice(0, 160) }) })
  }
  return attachedNow
}

// AO-SA-05 /sudo: the person types /sudo to a sub-agent addressed by id; the card must be the person's, once.
export async function sudoProbe(ctx) {
  const { a, R, attached, s, note } = ctx
  const target = (await a.rpc(listDescendantsRequest(R, ctx.orch)).then(r => r.AgentTree.tree.nodes, () => [])).find(n => n.alias === "k6")
  if (!target) return false
  a.rpc(submitPromptRequest(R, attached, target.agent_id, "/sudo Reply with the single word SUDOOK and nothing else.", [])).then(
    () => note("sudo-probe-answered", { agent: target.alias }), e => note("sudo-probe-refused", { message: e.message.slice(0, 200) }))
  note("sudo-probe-submitted", { agent: target.alias })
  return true
}

// AO-SA-04: the person answers a sub-agent's secret request; then the room agent tries the same card late.
export async function lateParentAnswer(ctx, interactionId, answeredBy) {
  const { a, R, orch, attached, note } = ctx
  await a.rpc(submitPromptRequest(R, attached, orch, `The person already answered the decision ${interactionId}. Run the room command \`agent answer ${interactionId} ok\` once and report the EXACT typed answer text. Then also run \`agent pass ${interactionId}\` once and report its exact answer. Do not retry. Reply LATEDONE.`, []))
    .then(() => note("late-parent-answer-prompted", { id: interactionId, answeredBy }), e => note("late-parent-answer-refused", { message: e.message.slice(0, 200) }))
}

// AO-PC-01 / AO-PE-04, phase B: after the orchestrator's tree exists, the person prompts agents that are NOT above c1.
export async function controlProbes(ctx) {
  const { a, R, attached, model, note, until } = ctx
  const nodes = (await a.rpc(listDescendantsRequest(R, ctx.orch))).AgentTree.tree.nodes
  const id = alias => nodes.find(n => n.alias === alias)?.agent_id
  const agents = (await a.rpc({ ListAgents: { session_id: R } })).AgentsListed.agents
  const peer = agents.find(x => x.alias === "peer")
  note("control-probes-targets", { c1: id("c1"), c2: id("c2"), m: id("m"), p: id("p"), peer: peer?.id })
  // another Room with its own agent q
  const other = (await a.rpc({ CreateSession: { workspace_id: ctx.work, worktree_id: ctx.work, alias: "other-room", agent_defaults: { provider: "claude", account_profile: ctx.profile, model, effort: "low", permission_level: "yolo" } } })).SessionCreated
  const R2 = other.session.id, q = other.agent.id
  const att2 = (await a.rpc({ AttachToSession: { session_id: R2, client_id: "orch-owner-r2", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  await a.client.subscribeToKernelEvents(R2, att2)
  await a.rpc({ LaunchProviderRun: { session_id: R2, agent_id: q, adapter_key: "claude", provider: "claude", account_profile: ctx.profile, model, variant: "low", native_tui: false } })
  note("other-room", { room: R2, agent: q })
  const report = `Report each exact answer as OK or REFUSED followed by the exact text. Do not retry a refused command. Do not wait for anything.`
  const sends = [
    // the peer (an agent of the person, beside the orchestrator) tries to control the orchestrator's children and the orchestrator itself
    [R, attached, peer?.id, `Run these room commands with the chariox room_run_command tool, one by one. ${report} (1) agent children c1 (2) agent deadline get c1 (3) agent cancel c1 (4) prompt c1 "steer from the peer" (5) agent delete c1 (6) agent children ${ctx.orch} (7) agent cancel ${ctx.orch} (8) agent answer no-such-decision ok (9) ${spawnLine("peerkid", model)} (10) prompt c1 "Run this shell command: echo PEERPROMPT > peer-wrote.txt" End with PEERDONE.`],
    // a sibling of c1 (c2) and c1 itself: sideways and upward
    [R, attached, id("c2"), `Run these room commands with the chariox room_run_command tool, one by one. ${report} (1) agent children c1 (2) agent cancel c1 (3) agent deadline get c1 (4) prompt c1 "steer from the sibling" (5) agent cancel ${ctx.orch} (6) prompt c1 "Run this shell command: echo SIBPROMPT > sib-wrote.txt". End with SIBDONE.`],
    [R, attached, id("c1"), `Run these room commands with the chariox room_run_command tool, one by one. ${report} (1) agent cancel q (2) agent children q (3) agent cancel c2 (4) agent spawn rr1 --provider claude --model ${model} --effort low --room (5) agent spawn rr2 --room (6) agent delete rr1 (7) agent delete rr2 (8) prompt ${ctx.orch} "Run this shell command: echo UPWARD > upward-wrote.txt". Then, if you have the chariox_kernel_request tool, call it once with request {"SetAgentParent":{"session_id":"${R}","agent_id":"${id("c2")}","parent_agent_id":"${id("c1")}"}} and report its exact answer; if you do not have that tool report NOTOOL. End with C1DONE.`],
    // a Required child asks its yolo sibling to run a shell command it would have to ask for itself
    [R, attached, id("p"), `Run this one room command with the chariox room_run_command tool and report its exact answer: prompt c1 "Run this shell command: echo BYPASS > bypass-wrote.txt" . Then reply PBYPASSDONE.`],
    // an agent of another room
    [R2, att2, q, `Run these room commands with the chariox room_run_command tool, one by one. ${report} (1) agent children c1 (2) agent children ${id("c1")} (3) agent cancel ${id("c1")} (4) agent deadline get ${id("c1")} (5) prompt ${id("c1")} "steer from another room" (6) agent delete ${id("c1")}. End with QDONE.`],
    // the room agent itself (a room agent may create peers, if the product has the verb)
    [R, attached, ctx.orch, `Run these room commands with the chariox room_run_command tool, one by one. ${report} (1) agent spawn roomagent-peer --provider claude --model ${model} --effort low --room (2) agent delete roomagent-peer. If (1) worked, also run: agent list. End with ROOMDONE.`],
  ]
  for (const [room, att, agent, text] of sends) {
    if (!agent) { note("control-probe-skipped", { reason: "no such agent" }); continue }
    await a.rpc(submitPromptRequest(room, att, agent, text, [])).then(() => note("control-probe-submitted", { agent }), e => note("control-probe-refused", { agent, message: e.message.slice(0, 200) }))
  }
  // Phase C: the grandparent controls a grandchild that is busy (AO-PC-01 allowed side).
  const g1 = nodes.find(n => n.alias === "g1")?.agent_id
  if (g1) {
    await a.rpc(submitPromptRequest(R, attached, g1, `Run this shell command and wait for it: until [ -e ${ctx.work}/never-appears-g1 ]; do sleep 1; done`, [])).then(() => note("grandchild-busy-prompted", { g1 }), e => note("grandchild-prompt-refused", { message: e.message.slice(0, 160) }))
    await delay(15000)
    await a.rpc(submitPromptRequest(R, attached, ctx.orch, `Run these room commands with the chariox room_run_command tool, one by one, and report each exact answer. Do not retry. (1) agent deadline g1 10m (2) agent deadline get g1 (3) agent cancel g1 (4) agent children. End with GRANDDONE.`, [])).then(() => note("grandparent-probe-submitted", {}), e => note("grandparent-probe-refused", { message: e.message.slice(0, 160) }))
  }
  // Without any agent token: the runtime tool endpoint of this kernel must not run a room command.
  const rawResult = await rawToolCall(ctx.mcpPort).catch(e => `error ${String(e.message).slice(0, 120)}`)
  note("non-agent-caller", { result: String(rawResult).slice(0, 300) })
  await delay(1000)
  return { R2, q }
}

async function rawToolCall(port) {
  const out = []
  for (const token of ["not-an-agent-token", ""]) {
    const res = await fetch(`http://127.0.0.1:${port}/mcp`, { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream", ...(token ? { authorization: `Bearer ${token}` } : {}) },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "chariox.room.run_command", arguments: { command: "agent children" } } }) })
    out.push(`${token ? "bogus token" : "no token"}: HTTP ${res.status} ${(await res.text()).slice(0, 100).replace(/\s+/g, " ")}`)
  }
  return out.join(" | ")
}

// AO-SA-04: a worker's decision goes to the parent that may answer it, a user-only one to the person as ONE card.
export function decisionsTwoBrief({ R, model, work }) {
  return `# Orchestrator brief (drill, decisions scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Use only your room commands (chariox room run_command) and chariox events tools. No /sudo. Do not ask the owner anything. Never retry a refused call more than once: record the exact answer. Never use shell sleeps or poll.
Keep ${join(work, "orchestrator-state.json")} updated after every step.
1. Spawn w1, w2, w3 with \`agent spawn <name> --provider claude --model ${model} --effort low --permission required\`. A worker at the required level asks before it acts: that is the point of the drill.
2. FIRST subscribe to w1, w2, w3 (chariox events_subscribe with each agent id), THEN send each text below to its worker with the room command \`prompt <alias> "<text>"\` (the worker runs it, you do not):
   w1: "Use your shell tool to run exactly: echo ALPHA > w1.txt . Then reply W1DONE."
   w2: "Call the chariox request_credential_secret tool with these arguments: credential {id 'drill-key', injection {kind 'header', name 'X-Drill-Key', value '{{secret}}'}, use_binding {target {kind 'site', origin 'https://example.invalid'}}}, prompt {message 'Drill: type any text', timeout_sec 900}. When it returns reply W2DONE."
   w3: "Use your shell tool to run exactly: echo GAMMA > w3.txt . Then reply W3DONE."
3. Yield on your registrations (events_subscriptions lists them) with a deadline 15 minutes ahead and END YOUR TURN. If events_yield is refused, do not retry: just END YOUR TURN, because every event wakes you anyway. Each worker decision that you may answer arrives as an urgent child_decision event (payload: interaction_id, child_agent_id, child_alias, question, choices, needs_answer).
4. On each wake: read the events (chariox events_inbox). For every child_decision event run the room command \`agent answer <interaction_id> <id of the choice that allows the action>\` (take the choice ids from the event) and append the interaction_id, the child_alias, the time and the EXACT typed answer to the state file. The answer may come back as settled_by_user or not_holder: record it and go on, never retry. Ack the events; for a child_finished event record the child and the outcome. Keep waking until you have seen the child_finished of w1, w2 and w3. If the person asks you to run a room command, run it once and report the exact answer. Do NOT write the report before the three finishes.
5. Then write ${join(work, "orchestrator-report.json")}: one entry per child_decision event (interaction_id, child_alias, answer status) and one per child_finished event (child, outcome kind). Note whether you ever received a child_decision for w2. Then delete w1, w2, w3 with \`agent delete <name>\` and reply ORCHDONE.
`
}
