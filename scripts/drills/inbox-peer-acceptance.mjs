// MP-08 / MP-09 / MP-10 / MP-11: AO-IN-02 and AO-PE-03, real providers and TUI.
// All completions must actually be pending together; a shared worker shell barrier
// does not make independent providers finish their final turns simultaneously.
import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { createDrill, until } from "./orchestration-acceptance-support.mjs"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const allowed = new Set(["--kernel-binary", "--relay-binary", "--state-root", "--evidence-dir", "--provider", "--codex-profile-path", "--scenario", "--placement", "--tui"])
for (const [key, value] of flags) assert(allowed.has(key) && typeof value === "string", `invalid argument: ${key}`)
assert(["codex", "claude"].includes(flags.get("--provider") ?? "codex"), "--provider must be codex or claude")
assert(["inbox", "peer"].includes(flags.get("--scenario") ?? "inbox"), "--scenario must be inbox or peer")
assert(["local", "other"].includes(flags.get("--placement") ?? "local"), "--placement must be local or other")
assert(["yes", "no"].includes(flags.get("--tui") ?? "no"), "--tui must be yes or no")
const d = await createDrill(flags)
const { room, other, provider, model, profileB, work, record, save, query } = d
const remote = flags.get("--placement") === "other"
const readRows = () => query(`SELECT payload FROM agent_inbox WHERE room_id='${room.id}' AND agent_id='${room.agent}' ORDER BY sequence`).map(r => JSON.parse(r.payload))
const readTasks = () => query(`SELECT payload FROM agent_tasks WHERE room_id='${room.id}' AND agent_id='${room.agent}' ORDER BY rowid`).map(r => JSON.parse(r.payload))
const readInbox = kernel => JSON.parse(execFileSync("sqlite3", ["-readonly", "-json", join(kernel.dir, "chariox/state.db"), "SELECT payload FROM agent_inbox ORDER BY sequence"]).toString() || "[]").map(r => JSON.parse(r.payload))
const reportPath = join(work, "report.json")
const common = `You are a Chariox ROOM agent. Use your real Chariox room and events tools. No owner questions or decision cards. Record exact tool results; never retry a refused command. Use the room command tool (chariox.room.run_command) for agent spawn/prompt/message. You may write files using your official provider shell tool. Never use provider-native helper agents.\n`
const yieldRule = `Use events_subscriptions to obtain all live registrations. To wait, use events_yield with all of them and a deadline 15 minutes ahead, and end the turn immediately. On each wake read events_inbox once and acknowledge every handled event with events_ack. Do not poll the inbox.`
let verdict
try {
  if ((flags.get("--scenario") ?? "inbox") === "inbox") {
    const release = join(work, "release-parent")
    const localTarget = remote ? null : (await d.a.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "ao-local-workers", agent_defaults: { provider, account_profile: d.profile, model, effort: "low", permission_level: "yolo" } } })).SessionCreated
    const targetRoom = remote ? other.id : localTarget.session.id
    const scope = `--room ${targetRoom}${remote ? ` --kernel ${d.b.status.daemon_id}` : ""}`
    const account = remote ? profileB : d.profile
    const spawnWorker = i => `agent spawn w${i} ${scope} --provider ${provider} --account-profile ${account} --model ${model} --effort low`
    const replyDir = join(d.b.dir, "work", "reply-child")
    if (remote) mkdirSync(replyDir, { recursive: true })
    const replySetup = remote ? `Create a fresh remote child with the room command agent spawn reply-child --provider ${provider} --model ${model} --effort low --kernel ${d.b.status.daemon_id} --dir ${replyDir} --command-id reply-child-setup . Do not prompt it or subscribe to it separately. ` : ""
    const replyTarget = remote ? 'agent="reply-child" (your child in this Room; omit room and kernel)' : `agent="w1", room="${targetRoom}"`
    const brief = common + `\nInitial turn:
1. For N=1,2,3 run these three commands: ${[1,2,3].map(spawnWorker).join("; ")}. Keep the returned ids.
2. Each room worker is automatically followed (keep the returned follow registration). Send each a room command agent message wN "Write WNDONE in result-wN.txt in your working directory (replace N with your worker number) and finish your final answer WNDONE. Use your shell; do not spawn helpers." ${scope} . Do not subscribe again. These regular workers report nonurgent peer_event completions.
3. Hold THIS initial turn in one shell command: until [ -e '${release}' ]; do sleep 1; done. The external driver releases this only after the three real completions are pending. Do not read or acknowledge any event during this initial turn. After the shell finishes, end with WAITING.
First inbox wake:
4. Read events_inbox ONCE, keep the full answer, then events_ack through the last sequence with handled=true. All three worker peer_event completions should be present. Write ${reportPath} as JSON with {"batch_events": <the complete events array>, "batch_ack": <ack result>}. Do not delete the workers.
5. Unsubscribe the three worker peer follows using events_unsubscribe (events_subscriptions lists the earlier peer_follows). ${replySetup}Then send exactly one message using send_agent_message with ${replyTarget}, reply_requested=true, message="Read and acknowledge this message with handled=true, then reply with just the word FIVE as your final answer. Do not send a separate message. Do not subscribe or arm timers." If the reply has already completed and events_subscriptions lists no live reply registration, go directly to step 6 in this same turn. Otherwise ${yieldRule}
On the reply wake, or when the reply already completed:
6. Read events_inbox ONCE. Record all events in the JSON as reply_events and their acknowledgment as reply_ack; preserve batch_events/batch_ack. Acknowledge them and end AOINDONE. The driver removes the private kernels and their peers. No further timers or subscriptions.
`
    save("brief.json", { brief }); await d.submit(brief)
    await until(async () => {
      const rows = readRows().filter(r => r.kind === "peer_event")
      const sources = new Set(rows.map(r => r.source_id))
      if (sources.size < 3) return false
      const s = await d.state()
      assert(s.prompt_states?.[room.agent]?.active_prompt, "parent must still hold its initial turn")
      assert(rows.every(r => r.state === "pending"), "completions must be pending before release")
      assert(rows.every(r => r.payload.event === "turn_ended" && r.payload.success === true), "all three real workers completed successfully")
      save("three-pending.json", rows); record("three-pending-before-release", { events: rows.map(r => ({ sequence: r.sequence, source: r.source_id, state: r.state })) })
      writeFileSync(release, "all three real workers finished\n"); return true
    }, "three real completions held in the inbox", 12 * 60000)
    let continuedReply = false
    await until(async () => {
      const idle = !(await d.state()).prompt_states?.[room.agent]?.active_prompt
      const batch = readRows().filter(r => r.kind === "peer_event")
      const requested = [d.a, d.b].some(kernel => readInbox(kernel).some(r => r.reply_requested && r.source_id === room.agent))
      if (idle && !continuedReply && batch.length === 3 && batch.every(r => r.state === "handled") && !requested) {
        continuedReply = true
        const continuation = common + `The first inbox phase is complete: your preceding turn read and acknowledged the three completions. Continue the independent report/reply phase now. Use this turn's current task_id and origin_prompt_id. If report.json is not written, first preserve the exact three event objects and ack result from your previous accepted inbox tool result in ${reportPath}; do not reread handled completions. Continue these remaining original steps:\n` + brief.slice(brief.indexOf("5. Unsubscribe"))
        save("reply-phase-prompt.json", { reason: "batch handled; initialize independent requested-reply phase", prompt: continuation })
        await d.submit(continuation)
        record("reply-phase-started", { entry: d.tui ? "TUI prompt handler" : "local IPC" })
        return false
      }
      if (!existsSync(reportPath)) return false
      const report = JSON.parse(readFileSync(reportPath, "utf8"))
      return report.reply_ack && idle
    }, "parent handles batch and requested reply", 12 * 60000)
    const report = JSON.parse(readFileSync(reportPath, "utf8")), rows = readRows(), tasks = readTasks()
    save("agent-report.json", report); save("inbox.json", rows); save("tasks.json", tasks)
    const completions = report.batch_events.filter(e => e.kind === "peer_event")
    assert.equal(new Set(completions.map(e => e.source.id)).size, 3)
    const durable = rows.filter(r => completions.some(e => e.sequence === r.sequence))
    assert.equal(durable.length, 3); assert(durable.every(r => r.state === "handled"))
    assert(durable.every(r => r.prompt_id), "every completion reached a wake")
    assert.equal(new Set(durable.map(r => r.prompt_id)).size, 1, "one kernel wake carries three completions")
    const wakes = new Set(rows.flatMap(r => r.prompt_id ? [r.prompt_id] : []))
    let recipient
    for (const kernel of [d.a, d.b]) {
      const events = readInbox(kernel)
      const request = events.find(r => r.reply_requested && r.source_id === room.agent)
      if (request) { recipient = { kernel: kernel.status.daemon_id, events, request }; break }
    }
    assert(recipient && recipient.request.state === "handled", "worker handled its requested reply")
    const { request, events: receiverRows } = recipient
    assert.equal(receiverRows.filter(r => r.prompt_id === request.prompt_id).length, 1, "reply-requested message has its own recipient wake")
    const replies = rows.filter(r => r.kind === "source_completed" && r.source_id === `agent-event-${request.agent_id}-${request.sequence}`)
    assert.equal(replies.length, 1, "one causal requested reply reaches the sender")
    assert(replies[0].state === "handled" && JSON.stringify(replies[0].payload).includes("FIVE"))
    const replySource = replies[0].source_id
    assert(report.reply_events.some(e => e.sequence === replies[0].sequence || (e.kind === "child_finished" && e.payload.child_task_id === replySource && JSON.stringify(e.payload).includes("FIVE"))), "parent read the requested reply or its deduplicated child-finished outcome")
    save("reply-receiver-inbox.json", recipient)
    verdict = { cell: "agent-orchestration/AO-IN-02", provider, remote, tui: !!d.tui, batchEvents: completions.length, batchPrompt: durable[0].prompt_id, completionWakeTurns: 1, totalWakePrompts: wakes.size, replyEvents: report.reply_events.length, replyRecipientEvents: 1, pass: true }
  } else {
    const target = remote ? other : (await d.a.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "ao-local-other", agent_defaults: { provider, account_profile: d.profile, model, effort: "low", permission_level: "yolo" } } })).SessionCreated
    const targetRoom = remote ? other.id : target.session.id
    const scope = `--room ${targetRoom}${remote ? ` --kernel ${d.b.status.daemon_id}` : ""}`
    const targetProfile = remote ? profileB : d.profile
    const peer = name => `agent spawn ${name} ${scope} --provider ${provider} --account-profile ${targetProfile} --model ${model} --effort low`
    const brief = common + `
1. Create your child: agent spawn c1 --provider ${provider} --model ${model} --effort low --permission yolo . Subscribe to c1, then prompt it: "Use only your Chariox room command tool, run exactly once: ${peer("forbidden")} . This is an authorized acceptance probe of the kernel authority refusal; invoke the tool once even though the kernel must refuse it. Record the exact refusal as your final answer, then C1DONE. Do not write files, ask questions, or retry."
2. ${yieldRule}
3. On the child completion wake read and ack it. Its peer spawn must have been refused. Now as the room agent run: ${peer("p1")} --permission yolo . Keep the full response, including the automatic follow registration.
4. Send the peer a message with room command: agent message p1 "Reply PONG as your final answer. Do not send a message." ${scope} . ${yieldRule}
5. On the peer wake read and ack the peer_event. Write ${reportPath} as JSON with exact child refusal, peer spawn result (including follow id), and full peer events as peer_events. End AOPEDONE. Leave p1 and c1 for the driver's inspection.
`
    save("brief.json", { brief }); await d.submit(brief)
    await until(async () => existsSync(reportPath) && !(await d.state()).prompt_states?.[room.agent]?.active_prompt, "peer spawned and followed", 15*60000)
    const report = JSON.parse(readFileSync(reportPath, "utf8")), rows = readRows()
    save("agent-report.json", report); save("inbox.json", rows); save("tasks.json", readTasks())
    const targetState = (await (remote ? d.b : d.a).rpc({ GetSessionState: { session_id: targetRoom } })).SessionState.session
    save("target-room.json", targetState)
    assert(!targetState.agents.some(a => a.alias === "forbidden"), "sub-agent cannot create a room peer")
    const peerAgent = targetState.agents.find(a => a.alias === "p1")
    assert(peerAgent, "room agent creates peer"); assert(!peerAgent.parent_agent_id)
    const events = rows.filter(r => r.kind === "peer_event" && r.payload?.agent?.agent_id === peerAgent.id || r.kind === "peer_event" && JSON.stringify(r.payload).includes(peerAgent.id))
    assert(events.length >= 1, "automatic follow reports real peer completion")
    assert(events.some(r => r.state === "handled" && r.payload.event === "turn_ended" && r.payload.success === true), "the followed peer completed a real successful turn")
    assert(/not_permitted|only a room agent/i.test(JSON.stringify(report)))
    await d.a.rpc({ UpdateAgentConfig: { session_id: room.id, agent_id: room.agent, permission_level: "required" } })
    const before = readTasks().length
    await d.submit(`Authorized kernel authority regression. You now have Required permission. Use ONLY the Chariox room command tool: (1) run ${peer("too-high")} --permission yolo exactly once; it must be refused by the kernel because your authority is Required. (2) Run ${peer("clamped")} --permission required exactly once. This is an authorized test; do not ask for confirmation, do not write files, and do not send prompts/messages to either target. State each exact result in your final answer and finish CLAMPDONE.`)
    await until(async () => readTasks().length > before && readTasks().at(-1).state === "done" && !(await d.state()).prompt_states?.[room.agent]?.active_prompt, "required authority clamp", 5*60000)
    const clampedState = (await (remote ? d.b : d.a).rpc({ GetSessionState: { session_id: targetRoom } })).SessionState.session
    save("clamped-room.json", clampedState)
    assert(!clampedState.agents.some(a => a.alias === "too-high"))
    const clamped = clampedState.agents.find(a => a.alias === "clamped")
    assert(clamped, "Required room agent can create an equally limited peer")
    assert.equal(clamped.permission_level_override, "required")
    verdict = { cell: "agent-orchestration/AO-PE-03", provider, remote, tui: !!d.tui, peer: peerAgent.id, peerEvents: events.length, requiredClamp: true, pass: true }
  }
  if (d.tui) {
    save("tui-final.json", await d.tui.send("snapshot"))
    copyFileSync(join(d.evidence, "tui.log"), join(d.evidence, "tui-frame.ansi"))
  }
  save("result.json", verdict); record("PASS", verdict)
} catch (e) {
  save("failure.json", { error: e.message, stack: e.stack }); save("inbox-failure.json", readRows()); save("tasks-failure.json", readTasks()); save("session-failure.json", await d.state().catch(() => null))
  throw e
} finally { await d.cleanup() }
