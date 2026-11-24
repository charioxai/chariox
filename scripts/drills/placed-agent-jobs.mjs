// Run under a private systemd unit with a clean environment and an approved provider account.
import assert from "node:assert/strict"
import { existsSync, mkdirSync, readFileSync } from "node:fs"
import { join } from "node:path"
import { getSessionHistoryOutlineRequest } from "../../packages/kernel-client/dist/ipc-recall-requests.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { createDrill, until } from "./orchestration-acceptance-support.mjs"

const flags = new Map()
for (let i = 2; i < process.argv.length; i += 2) flags.set(process.argv[i], process.argv[i + 1])
if (!flags.has("--provider")) flags.set("--provider", "claude")
assert(["claude", "codex"].includes(flags.get("--provider")))
const drill = await createDrill(flags, {
  captureLedgers: false,
  copyClaudeAccount: false,
  kernelEnvironment: { CHARIOX_ROOM_AGENT_TOOLS: "1" },
})
const { a, b, room, provider, model, profile, record, save } = drill
const workspace = join(b.dir, "work", "placed")
const startPath = join(workspace, "started.json")
const wakePath = join(workspace, "completion.json")
const stampPath = join(workspace, "worker-finished")
const expected = "placed-job-completed"
const isRed = flags.get("--expect") === "red"

try {
  mkdirSync(workspace)
  const agent = (await a.rpc({ SpawnAgent: {
    session_id: room.id, alias: "placed", provider, model, effort: "low",
    permission_level: "yolo", account_profile: profile, kernel_ref: b.status.daemon_id,
    worktree_id: workspace,
  } })).AgentSpawned.agent
  record("placed-agent", { id: agent.id, worker: b.status.daemon_id, workspace })
  if (provider === "claude") {
    // Confirm the selected portable account through the Room's profile update.
    await a.rpc({ UpdateAgentProfile: {
      session_id: room.id, agent_id: agent.id, account_profile: profile, effort: "low",
    } })
    record("worker-profile-confirmed")
  }
  const brief = `On this initial turn, use the Chariox jobs.run tool with command ["/bin/sh","-c","sleep 300; printf placed-job-completed > worker-finished; printf 'placed-job-completed\\n'"] and wait_s=0. Supply this turn's task_id and origin_prompt_id. On success, use jobs.status and jobs.list once each. Also try jobs.run with command ["/bin/true"] and cwd "missing-for-drill", and keep its refusal. Write started.json in the current directory using your shell, with keys run, status, list, refused and turn. Copy the exact structured tool results for the first four keys and set turn to this turn's origin_prompt_id. If the first request failed, write just run and turn. Finish this turn immediately after writing the report. Do not start the command a second time or use a timer.

When Chariox wakes you for job completion, use this new turn's IDs. Read events.inbox using the completion event sequence from the wake, if it names a sequence. Use jobs.status once with the earlier job ID. Write completion.json with keys inbox, status and turn. Copy the exact structured results and this new turn's origin_prompt_id. Acknowledge the event if it still needs acknowledgment, then finish.`
  save("brief.json", { brief })
  save("submitted.json", await a.rpc(submitPromptRequest(room.id, room.attachment, agent.id, brief, [])))
  try {
    await until(() => existsSync(startPath), "placed agent reports job tools", 120000)
  } catch (error) {
    const session = await drill.state()
    save("turn-state.json", session.prompt_states?.[agent.id] ?? null)
    save("turn-history.json", await a.rpc(getSessionHistoryOutlineRequest(room.id, [agent.id], 2, null, true)))
    throw error
  }
  const started = JSON.parse(readFileSync(startPath, "utf8"))
  save("started.json", started)
  if (isRed) {
    assert.match(JSON.stringify(started.run), /jobs require the home agent's active build run/)
    assert.match(JSON.stringify(started.run), /retryable=true/)
    record("base-refuses-placed-job")
  } else {
    const id = started.run.job_id
    assert.match(id, /^job-/)
    assert.equal(started.status.job_id, id)
    assert(started.list.jobs.some(job => job.job_id === id))
    record("J2", { id })
    assert.equal(started.refused.retryable, false)
    assert.match(started.refused.error, /directory inside the agent workspace/)
    record("J4", started.refused)

    await until(async () => {
      const session = await drill.state()
      return !session.prompt_states?.[agent.id]?.active_prompt
    }, "initial placed turn has ended", 60000)
    assert(!existsSync(stampPath), "command is still running after the first turn settled")
    record("initial-turn-ended-before-job", { turn: started.turn })

    await until(() => existsSync(wakePath), "placed agent answers the completion wake", 420000)
    const finished = JSON.parse(readFileSync(wakePath, "utf8"))
    save("completion.json", finished)
    assert.equal(readFileSync(stampPath, "utf8"), expected)
    assert.equal(finished.status.job_id, id)
    assert.equal(finished.status.result.exit_code, 0)
    assert(finished.status.result.duration_ms >= 299000, "job ran for five minutes")
    assert.match(finished.status.result.output_tail, /placed-job-completed/)
    const events = finished.inbox.events
    assert(events.some(event => {
      const job = event.payload?.public_answer ?? event.payload
      return job?.job_id === id && job.result?.exit_code === 0
    }))
    assert.notEqual(finished.turn, started.turn)
    record("J1", { id, worker: b.status.daemon_id, result: finished.status.result })
    record("J3", { firstTurn: started.turn, completionTurn: finished.turn })
  }
} finally {
  await drill.cleanup()
}
