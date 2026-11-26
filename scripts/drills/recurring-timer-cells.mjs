// Row 554 cells d/e/f (orch15's evidence): a RECURRING timer (interval 2 min) armed by a real Claude agent before a hot reload (or a
// cold restart) keeps firing after it: fire_count rises and next_due_ms stays ahead of the clock, never stuck in the past.
//   d1  the agent arms the timer and ends its turn.
//   d2  the agent arms the timer, then waits (yields) on another timer, so its turn stays open while the recurring one fires.
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
  await until(async () => !!(await wakeOf(d1, "NW554-REC1")) && !!(await wakeOf(d2, "NW554-REC2")) && !!(await wakeOf(d2, "NW554-HOLD")), "both recurring timers armed", 300000)
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
  for (const [name, label] of [["d1", "NW554-REC1"], ["d2", "NW554-REC2"]]) {
    const mine = samples.filter(s => s.name === name)
    const last = mine.at(-1)
    const fires = last ? last.fire_count - before[name].fire_count : 0
    const ok = !!last && fires >= 2 && last.next_due_in_s > -30 && last.state === "scheduled"
    cell(`${name}-a-recurring-timer-armed-before-the-${cold ? "cold-restart" : "hot-reload"}-keeps-firing`, ok, { fires_after: fires, last })
  }
} catch (error) {
  k.note("error", { message: String(error.message ?? error).slice(0, 400) })
  exitCode = 1
} finally {
  await k.stop()
}
process.exit(exitCode)
