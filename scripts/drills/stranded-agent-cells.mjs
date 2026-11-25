// Row 554 live cells, one real kernel and real Claude agents (own state, never the lane kernel). The account comes through
// the product setup-token flow: a drill Vault, then SetProviderAccountCredential with the authorized token file read in
// memory (never printed, never in argv or logs).
//   1  a command the agent leaves running in the background at turn end is adopted as a wake; its end wakes the agent.
//   2  a turn that ends saying it waits, with nothing armed, gets a backstop timer (30 min); the timer wakes the agent.
//      A second agent on the native TUI says whether its answer text reaches the history the stated-wait read uses.
//   3  no turn end is refused: every turn above completes.
//   4  a non-urgent agent message to an idle agent with nothing armed starts its turn.
// Cells 1 and 4 run before and after a hot reload (SIGUSR2); cell 2's backstop is armed before it and must survive it.
// Run: node scripts/drills/stranded-agent-cells.mjs --kernel-binary <path> --token-file <file>
//   --state-root /root/.chariox/dev/<lane>/state --evidence-dir <dir> [--model claude-haiku-5-5] [--skip-backstop-wait 1]
import { randomBytes } from "node:crypto"
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { listDescendantsRequest } from "../../packages/kernel-client/dist/ipc-agent-requests.js"
import { drillFlags, startDrillKernel, until } from "./own-kernel.mjs"

const { flags, need } = drillFlags(process.argv)
const evidence = need("--evidence-dir")
const model = flags.get("--model") ?? "claude-haiku-5-5"
const tokenFile = need("--token-file")
const skipBackstopWait = flags.get("--skip-backstop-wait") === "1"
const k = await startDrillKernel({ label: "nowake554", binary: need("--kernel-binary"), stateRoot: need("--state-root"), evidence, extraEnv: { CHARIOX_ROOM_AGENT_TOOLS: "1" } })
const results = []
const cell = (id, pass, detail = {}) => {
  results.push({ id, status: pass === null ? "INCONCLUSIVE" : pass ? "PASS" : "FAIL", ...detail })
  writeFileSync(join(evidence, "cells.json"), JSON.stringify(results, null, 1) + "\n", { mode: 0o600 })
  console.log(id, results.at(-1).status, JSON.stringify(detail))
}
const save = (name, value) => writeFileSync(join(evidence, name), JSON.stringify(value, null, 1), { mode: 0o600 })
let exitCode = 0
const live = wake => ["scheduled", "starting", "running"].includes(wake.state)
try {
  const passphrase = randomBytes(32).toString("hex")
  await k.rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase } } })
  await k.rpc({ UnlockCredentialVault: { passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
  await k.rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: readFileSync(tokenFile, "utf8").trim(), overwrite: true } })
  const profile = (await k.rpc({ GetProviderAccountProfile: { provider: "claude", account_profile: "default" } })).ProviderAccountProfile.profile.profile_id
  k.note("account-stored", { profile })

  async function room(alias, nativeTui = false) {
    const work = join(k.dir, "work", alias)
    mkdirSync(work, { recursive: true })
    const made = (await k.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias, agent_defaults: { provider: "claude", account_profile: profile, model, effort: "low", permission_level: "yolo" } } }))
    const R = made.SessionCreated.session.id, agent = made.SessionCreated.agent.id
    const attached = (await k.rpc({ AttachToSession: { session_id: R, client_id: `nowake554-${alias}`, capability_level: "FullTerminal" } })).SessionAttached.attachment.id
    await k.client.subscribeToKernelEvents(R, attached)
    await k.rpc({ LaunchProviderRun: { session_id: R, agent_id: agent, adapter_key: "claude", provider: "claude", account_profile: profile, model, variant: "low", native_tui: nativeTui } })
    return { R, agent, attached, work }
  }
  const turnsOf = async (R, agent) => (await k.rpc({ GetSessionHistoryOutline: { session_id: R, agent_ids: [agent], latest_prompt_count: 40, include_prefetched_content: true } })).SessionHistoryOutline.agents[0]?.turns ?? []
  const done = turns => turns.filter(turn => turn.lifecycle === "completed")
  const text = turn => JSON.stringify(turn.summary ?? turn).slice(0, 4000)
  const wakesOf = async (R, agent) => ((await k.session(R)).agent_wakes ?? []).filter(w => w.agent_id === agent)
  const prompt = (r, agent, ask) => k.rpc(submitPromptRequest(r.R, r.attached, agent, ask, []))
  const attempt = async (id, run) => {
    try { await run() } catch (error) { cell(id, false, { refused: String(error.message ?? error).slice(0, 400) }); exitCode = 1 }
  }

  // Cell 1 (and 3): the adopted background command.
  const adopt = suffix => attempt(`1${suffix}`, async () => {
    const r = await room(`c1${suffix}`)
    const marker = join(r.work, "l1.done")
    await prompt(r, r.agent, `Run exactly this one command with the Bash tool, NOT in the background mode: nohup sh -c 'sleep 90; echo L1-DONE > ${marker}' >/dev/null 2>&1 &   Then end your turn at once with the single word STARTED. Do not wait, poll or sleep. If you are later woken because that background command ended, reply with the single word L1-WOKE and nothing else.`)
    await until(async () => done(await turnsOf(r.R, r.agent)).length >= 1, "the launching turn completes", 240000)
    const adopted = await until(async () => (await wakesOf(r.R, r.agent)).find(w => w.kind === "adopted_process" && live(w)), "the command is adopted", 30000).catch(() => null)
    const ended = adopted && await until(() => existsSync(marker), "the command ends", 240000).then(() => true, () => false)
    const woke = ended && await until(async () => done(await turnsOf(r.R, r.agent)).some(turn => text(turn).includes("L1-WOKE")), "the wake turn runs", 240000).then(() => true, () => false)
    save(`turns-1${suffix}.json`, await turnsOf(r.R, r.agent))
    save(`wakes-1${suffix}.json`, await wakesOf(r.R, r.agent))
    cell(`1${suffix}-background-command-adopted-and-woken`, !!woke, { adopted: adopted ? adopted.label : null, command_ended: !!ended })
    if (!woke) exitCode = 1
  })

  // Cell 4: a non-urgent message to an idle agent with nothing armed.
  const message = suffix => attempt(`4${suffix}`, async () => {
    const r = await room(`c4${suffix}`)
    await prompt(r, r.agent, `Run this room command: agent spawn kid --provider claude --model ${model} --effort low . Then run the room command: prompt kid "Reply with the single word KID-READY and end your turn." . Then end your turn with the single word SPAWNED. When you are woken because kid finished, call the chariox send_agent_message tool once to agent kid with the message "L4-GO: reply with the single word KID-GOT-IT and nothing else" and do NOT set urgent. Then end your turn with the single word SENT.`)
    const kid = await until(async () => {
      const tree = await k.rpc(listDescendantsRequest(r.R, r.agent)).then(x => x.AgentTree.tree, () => null)
      const node = tree?.nodes.find(n => n.alias === "kid")
      return node ? { id: node.agent_id } : null
    }, "the child exists", 240000).catch(() => null)
    if (!kid) { cell(`4${suffix}-message-starts-the-idle-agent`, false, { why: "the parent never spawned the child" }); exitCode = 1; return }
    await until(async () => done(await turnsOf(r.R, kid.id)).length >= 1, "the child's first turn completes", 240000)
    const idleAt = Date.now()
    const got = await until(async () => done(await turnsOf(r.R, kid.id)).some(turn => text(turn).includes("KID-GOT-IT")), "the message starts the idle child's turn", 300000).then(() => true, () => false)
    save(`turns-4${suffix}-kid.json`, await turnsOf(r.R, kid.id))
    save(`turns-4${suffix}-parent.json`, await turnsOf(r.R, r.agent))
    cell(`4${suffix}-message-starts-the-idle-agent`, got, { seconds_after_idle: Math.round((Date.now() - idleAt) / 1000) })
    if (!got) exitCode = 1
  })

  // Cell 2: the backstop, armed before the hot reload.
  let backstop = null
  const stated = async (alias, nativeTui) => {
    const r = await room(alias, nativeTui)
    await prompt(r, r.agent, "Reply with exactly this one sentence and nothing else: Pushed the change; waiting for the review.")
    await until(async () => done(await turnsOf(r.R, r.agent)).length >= 1, `${alias} turn completes`, 240000)
    const wake = await until(async () => (await wakesOf(r.R, r.agent)).find(w => w.kind === "timer" && live(w) && String(w.label).startsWith("backstop for a stated wait")), "the backstop is armed", 30000).catch(() => null)
    save(`turns-${alias}.json`, await turnsOf(r.R, r.agent))
    save(`wakes-${alias}.json`, await wakesOf(r.R, r.agent))
    return { r, wake }
  }
  await attempt("2", async () => {
    const s = await stated("c2", false)
    cell("2-stated-wait-gets-a-backstop", !!s.wake, { label: s.wake?.label ?? null, due_ms: s.wake?.next_due_ms ?? null })
    if (!s.wake) { exitCode = 1; return }
    backstop = { ...s, id: s.wake.id }
  })
  await attempt("2-native", async () => {
    const s = await stated("c2n", true)
    // Informational: whether a native-TUI answer's text reaches the history the stated-wait read uses.
    cell("2-native-tui-answer-text-reaches-the-stated-wait-read", s.wake ? true : null, { backstop_armed: !!s.wake, label: s.wake?.label ?? null })
  })

  await adopt("")
  await message("")

  // The hot reload: SIGUSR2 replaces the program in place and keeps the PID.
  const pid = k.pid
  if (pid) {
    process.kill(pid, "SIGUSR2")
    k.note("hot-reload-requested", { pid })
    await until(async () => { try { return !!(await k.rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, "kernel back after the hot reload", 180000)
    const kept = backstop ? (await wakesOf(backstop.r.R, backstop.r.agent)).find(w => w.id === backstop.id && live(w)) : null
    cell("2-backstop-survives-the-hot-reload", backstop ? !!kept : null, { kept: kept ? kept.state : null })
    if (backstop && !kept) exitCode = 1
    await adopt("-after-reload")
    await message("-after-reload")
  } else {
    cell("hot-reload", null, { why: "the drill kernel pid is not exposed" })
  }

  // The backstop is the only wake left; its firing is the agent's next turn.
  if (backstop && !skipBackstopWait) {
    const due = backstop.wake.next_due_ms
    const woke = await until(async () => done(await turnsOf(backstop.r.R, backstop.r.agent)).some(turn => /backstop for a stated wait/.test(text(turn)) && /Wake|wake/.test(text(turn)) && !/Kernel: your turn ended/.test(text(turn).slice(0, 80))), "the backstop wakes the agent", Math.max(60000, due - Date.now() + 240000)).then(() => true, () => false)
    save("turns-c2-final.json", await turnsOf(backstop.r.R, backstop.r.agent))
    cell("2-backstop-wakes-the-agent", woke, { late_s: Math.round((Date.now() - due) / 1000) })
    if (!woke) exitCode = 1
  }
  cell("3-no-turn-end-refused", results.every(c => c.status !== "FAIL") ? true : null, { note: "every turn above ended and completed" })
} catch (error) {
  k.note("error", { message: String(error.message ?? error).slice(0, 400) })
  exitCode = 1
} finally {
  await k.stop()
}
process.exit(exitCode)
