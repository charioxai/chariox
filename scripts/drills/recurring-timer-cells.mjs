// Row 554 cells d/e/f (orch15's evidence): a RECURRING timer (interval 2 min) armed by a real Claude agent before a hot reload (or a
// cold restart) fires on time after it and is never stuck: scheduled with next_due_ms ahead of the clock, or ended.
//   d1  the agent arms the timer and ends its turn: the kernel's one-fire contract (#1731) applies: it fires once after the restart,
//       is delivered, and ends (`fired`).
//   d2  the agent arms the timer, then waits (yields) on another timer: the timer fires on time after the restart; it stays scheduled
//       while the wait lasts, and ends only with the task (the wake's own turn is a successor and retires the wait).
// Run: node scripts/drills/recurring-timer-cells.mjs --kernel-binary <path> --token-file <file> --state-root <dir> --evidence-dir <dir> [--cold 1]
import { randomBytes } from "node:crypto"
import { mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { drillFlags, startDrillKernel, until } from "./own-kernel.mjs"

const { flags, need } = drillFlags(process.argv)
const evidence = need("--evidence-dir")
const cold = flags.get("--cold") === "1"
const observeMs = Number(flags.get("--observe-ms") ?? 480000)
const k = await startDrillKernel({ label: "nw554rec", binary: need("--kernel-binary"), stateRoot: need("--state-root"), evidence, extraEnv: { CHARIOX_ROOM_AGENT_TOOLS: "1" } })
const results = []
let exitCode = 0
const cell = (id, pass, detail = {}) => {
  results.push({ id, status: pass === null ? "INCONCLUSIVE" : pass ? "PASS" : "FAIL", ...detail })
  if (pass === false) exitCode = 1
  writeFileSync(join(evidence, "cells.json"), JSON.stringify(results, null, 1) + "\n", { mode: 0o600 })
  console.log(id, results.at(-1).status, JSON.stringify(detail))
}
try {
  const passphrase = randomBytes(32).toString("hex")
  await k.rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase } } })
  await k.rpc({ LockCredentialVault: null })
  await k.rpc({ UnlockCredentialVault: { passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
  await k.rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: readFileSync(need("--token-file"), "utf8").trim(), overwrite: true } })
  const profile = (await k.rpc({ GetProviderAccountProfile: { provider: "claude", account_profile: "default" } })).ProviderAccountProfile.profile.profile_id
  const model = flags.get("--model") ?? "claude-haiku-5-5"
  const room = async alias => {
    const work = join(k.dir, "work", alias)
    mkdirSync(work, { recursive: true })
    const made = await k.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias, agent_defaults: { provider: "claude", account_profile: profile, model, effort: "low", permission_level: "yolo" } } })
    const R = made.SessionCreated.session.id, agent = made.SessionCreated.agent.id
    const attached = (await k.rpc({ AttachToSession: { session_id: R, client_id: `nw554rec-${alias}`, capability_level: "FullTerminal" } })).SessionAttached.attachment.id
    await k.client.subscribeToKernelEvents(R, attached)
    await k.rpc({ LaunchProviderRun: { session_id: R, agent_id: agent, adapter_key: "claude", provider: "claude", account_profile: profile, model, variant: "low", native_tui: false } })
    return { R, agent, attached }
  }
  const turnsOf = async r => (await k.rpc({ GetSessionHistoryOutline: { session_id: r.R, agent_ids: [r.agent], latest_prompt_count: 40, include_prefetched_content: true } })).SessionHistoryOutline.agents[0]?.turns ?? []
  const wakeOf = async (r, label) => (((await k.session(r.R)).agent_wakes ?? []).filter(w => w.agent_id === r.agent && w.label === label))[0]
  const ask = (r, text) => k.rpc(submitPromptRequest(r.R, r.attached, r.agent, text, []))
  const d1 = await room("d1"), d2 = await room("d2")
  await ask(d1, 'Call the chariox events_timer tool once with delay_ms 120000, interval_ms 120000 and label "NW554-REC1". Do NOT call events_yield and do not wait. End your turn at once with the single word ARMED. If a wake for NW554-REC1 arrives later, reply with the single word TICK.')
  await ask(d2, 'Call the chariox events_timer tool once with delay_ms 120000, interval_ms 120000 and label "NW554-REC2". Then call events_timer once more with delay_ms 1500000 and label "NW554-HOLD", then call events_yield waiting on the NW554-HOLD registration only. Do not answer until it wakes you.')
  // Restart only when the agents are in the states the cells are about: d1's turn completed, d2's task waiting.
  const taskStates = async r => ((await k.session(r.R)).agent_tasks ?? []).filter(t => t.agent_id === r.agent).map(t => t.state)
  await until(async () => !!(await wakeOf(d1, "NW554-REC1")) && !!(await wakeOf(d2, "NW554-REC2")) && !!(await wakeOf(d2, "NW554-HOLD")), "both recurring timers armed", 300000)
  await until(async () => (await turnsOf(d1)).some(turn => turn.lifecycle === "completed") && (await taskStates(d2)).includes("waiting"), "d1's turn completed and d2's task waiting", 300000)
  const before = { d1: await wakeOf(d1, "NW554-REC1"), d2: await wakeOf(d2, "NW554-REC2") }
  k.note("armed", { d1_interval: before.d1.interval_ms, d2_interval: before.d2.interval_ms, d1_due: before.d1.next_due_ms })
  if (cold) {
    await k.coldRestart("SIGKILL")
    await k.rpc({ UnlockCredentialVault: { passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
    k.note("vault-unlocked-after-the-cold-restart", {})
  } else {
    process.kill(k.pid, "SIGUSR2")
    k.note("hot-reload-requested", { pid: k.pid })
    await until(async () => { try { return !!(await k.rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, "kernel back", 180000)
  }
  const restartedAt = Date.now()
  const samples = []
  while (Date.now() - restartedAt < observeMs) {
    await delay(30000)
    for (const [name, r, label] of [["d1", d1, "NW554-REC1"], ["d2", d2, "NW554-REC2"]]) {
      let w; try { w = await wakeOf(r, label) } catch { continue }
      if (w) samples.push({ at: Math.round((Date.now() - restartedAt) / 1000), name, state: w.state, fire_count: w.fire_count, next_due_in_s: Math.round((w.next_due_ms - Date.now()) / 1000), last_delivery: w.last_delivery })
    }
    writeFileSync(join(evidence, "samples.json"), JSON.stringify(samples, null, 1))
  }
  // Never stuck: scheduled with a due time ahead of the clock, or ended; never scheduled and overdue.
  const neverStuck = list => list.every(x => x.state !== "scheduled" || x.next_due_in_s > -60)
  const mine1 = samples.filter(x => x.name === "d1"), mine2 = samples.filter(x => x.name === "d2")
  const last1 = mine1.at(-1), last2 = mine2.at(-1)
  const fires1 = last1 ? last1.fire_count - before.d1.fire_count : 0
  const fires2 = last2 ? last2.fire_count - before.d2.fire_count : 0
  const mode = cold ? "cold-restart" : "hot-reload"
  cell(`d1-a-timer-left-by-an-ended-turn-fires-once-after-the-${mode}-and-ends`, !!last1 && fires1 === 1 && last1.state === "fired" && last1.last_delivery === "handled", { fires_after: fires1, last: last1 })
  cell(`d2-a-recurring-timer-of-a-waiting-task-fires-on-time-after-the-${mode}-and-is-never-stuck`, !!last2 && fires2 >= 1 && neverStuck(mine2), { fires_after: fires2, last: last2 })
} catch (error) {
  k.note("error", { message: String(error.message ?? error).slice(0, 400) })
  exitCode = 1
} finally {
  await k.stop()
}
process.exit(exitCode)
